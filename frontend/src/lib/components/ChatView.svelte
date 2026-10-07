<script lang="ts">
	import { goto } from '$app/navigation';
	import { get, patch, post, errorMessage } from '#lib/api.ts';
	import { postSSE } from '#lib/sse.ts';
	import { session } from '#lib/session.svelte.ts';
	import { loadModels, pendingSend, refreshChats, store } from '#lib/chats.svelte.ts';
	import { fmtCost, fmtNum } from '#lib/format.ts';
	import ModelPicker from './ModelPicker.svelte';
	import Composer from './Composer.svelte';
	import Message from './Message.svelte';
	import Modal from './Modal.svelte';

	let { chatId = null }: { chatId?: string | null } = $props();

	let chat = $state<any>(null);
	let messages = $state<any[]>([]);
	let totals = $state<any>(null);
	let modelId = $state<string | null>(null);
	let busy = $state(false);
	let error = $state('');
	let controller: AbortController | null = null;
	let scroller: HTMLDivElement;
	let stick = true;
	let settingsOpen = $state(false);
	let cs = $state({ system_prompt: '', temperature: '', max_tokens: '' });
	let showMetrics = $state(true);

	const model = $derived(store.models.find((m) => m.id === modelId));
	const webSearchAvailable = $derived(store.skills.includes('web_search') && !!model?.supports_tools);
	const webSearchOn = $derived(!!chat?.skills?.includes('web_search') || !!model?.forced_skills.includes('web_search'));
	const cacheRate = $derived(totals?.cache_reported && totals.input_tokens ? (100 * (totals.cached_tokens ?? 0)) / totals.input_tokens : null);

	try {
		const v = localStorage.getItem('showMetrics');
		showMetrics = v === null ? (session.me?.show_metrics ?? true) : v === '1';
	} catch {}

	function pickDefaultModel() {
		if (modelId && store.models.some((m) => m.id === modelId)) return;
		const lastUsed = [...messages].reverse().find((m) => m.role === 'assistant' && m.model_id)?.model_id;
		const candidates = [lastUsed, session.me?.default_model_id, store.models[0]?.id];
		modelId = candidates.find((id) => id && store.models.some((m) => m.id === id)) ?? null;
	}

	async function load() {
		if (!store.modelsLoaded) await loadModels().catch((e) => (error = errorMessage(e)));
		if (chatId) {
			try {
				const r = await get(`/api/chats/${chatId}`);
				chat = r.chat;
				messages = r.messages;
				totals = r.totals;
			} catch (e) {
				error = errorMessage(e);
				return;
			}
		}
		pickDefaultModel();
		scrollToBottom(true);
		if (chatId && pendingSend.chatId === chatId) {
			const p = { ...pendingSend };
			pendingSend.chatId = null;
			modelId = p.modelId;
			run({ content: p.content, attachments: p.attachments });
		}
	}
	load();

	function scrollToBottom(force = false) {
		requestAnimationFrame(() => {
			if (scroller && (force || stick)) scroller.scrollTop = scroller.scrollHeight;
		});
	}

	function onscroll() {
		stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
	}

	function addTotals(m: any) {
		if (!m) return;
		totals ??= {};
		for (const k of ['input_tokens', 'output_tokens', 'cached_tokens', 'cache_write_tokens', 'reasoning_tokens', 'cost']) {
			if (m[k] != null) totals[k] = (totals[k] ?? 0) + m[k];
		}
		if (m.cached_tokens != null) totals.cache_reported = true;
	}

	async function send(content: string, attachments: any[]) {
		if (!modelId) return;
		if (!chatId) {
			try {
				const c = await post('/api/chats');
				Object.assign(pendingSend, { chatId: c.id, content, attachments, modelId });
				await refreshChats();
				goto(`/c/${c.id}`);
			} catch (e) {
				error = errorMessage(e);
			}
			return;
		}
		run({ content, attachments });
	}

	async function run(body: { content?: string; attachments?: any[]; regenerate?: boolean; edit_message_id?: string }) {
		if (!chatId || !modelId || busy) return;
		error = '';
		busy = true;
		stick = true;
		controller = new AbortController();
		if (body.regenerate) {
			while (messages.length && messages.at(-1).role === 'assistant') messages.pop();
		}
		if (body.edit_message_id) {
			const i = messages.findIndex((m) => m.id === body.edit_message_id);
			if (i >= 0) messages.splice(i);
		}
		const draftIndex = { value: -1 };
		const draft = () => messages[draftIndex.value];
		try {
			await postSSE(`/api/chats/${chatId}/messages`, { model_id: modelId, ...body }, controller.signal, (event, data) => {
				switch (event) {
					case 'user_message':
						messages.push(data);
						break;
					case 'title':
						chat.title = data.title;
						refreshChats();
						break;
					case 'start':
						messages.push({ id: null, role: 'assistant', model_id: data.model_id, model_name: data.model_name, content: '', reasoning: '', tool_steps: [], sources: [], streaming: true });
						draftIndex.value = messages.length - 1;
						break;
					case 'delta':
						draft().content += data.text;
						break;
					case 'reasoning':
						draft().reasoning += data.text;
						break;
					case 'tool_call':
						draft().tool_steps.push({ id: data.id, name: data.name, pending: true, summary: data.name === 'web_search' ? `Searching: ${JSON.parse(data.arguments || '{}').query ?? ''}` : `Reading page…` });
						break;
					case 'tool_result': {
						const steps = draft().tool_steps;
						const i = steps.findIndex((s: any) => s.id === data.id);
						steps[i >= 0 ? i : steps.length] = data;
						break;
					}
					case 'done':
						messages[draftIndex.value] = data.message;
						addTotals(data.message.metrics);
						break;
					case 'error':
						if (data.saved) messages[draftIndex.value] = data.saved;
						else if (draft()) Object.assign(draft(), { streaming: false, error: data.message });
						else error = data.message;
						break;
				}
				scrollToBottom();
			});
		} catch (e) {
			if ((e as Error).name === 'AbortError') {
				if (draft()) Object.assign(draft(), { streaming: false, error: 'Stopped' });
				setTimeout(async () => {
					const r = await get(`/api/chats/${chatId}`);
					messages = r.messages;
				}, 500);
			} else {
				error = errorMessage(e);
				if (draft()) draft().streaming = false;
			}
		} finally {
			busy = false;
			controller = null;
			refreshChats();
		}
	}

	async function toggleWebSearch() {
		if (!chat) return;
		const skills = webSearchOn ? chat.skills.filter((s: string) => s !== 'web_search') : [...chat.skills, 'web_search'];
		chat = await patch(`/api/chats/${chatId}`, { skills });
	}

	function openSettings() {
		cs = { system_prompt: chat.system_prompt ?? '', temperature: chat.temperature ?? '', max_tokens: chat.max_tokens ?? '' };
		settingsOpen = true;
	}

	async function saveSettings(e: SubmitEvent) {
		e.preventDefault();
		try {
			chat = await patch(`/api/chats/${chatId}`, {
				system_prompt: cs.system_prompt || null,
				temperature: cs.temperature === '' ? null : Number(cs.temperature),
				max_tokens: cs.max_tokens === '' ? null : Number(cs.max_tokens)
			});
			try { localStorage.setItem('showMetrics', showMetrics ? '1' : '0'); } catch {}
			settingsOpen = false;
		} catch (err) {
			error = errorMessage(err);
		}
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && busy) controller?.abort();
	}
</script>

<svelte:window {onkeydown} />

<div class="flex h-full flex-col">
	<header class="flex flex-wrap items-center gap-2 border-b border-zinc-200 px-4 py-2 dark:border-zinc-800">
		<ModelPicker bind:value={modelId} />
		{#if chat}
			<h1 class="min-w-0 flex-1 truncate text-sm text-zinc-500" title={chat.title}>{chat.title}</h1>
			{#if totals && (totals.input_tokens != null || totals.output_tokens != null)}
				<div class="hidden items-center gap-2 text-xs text-zinc-500 md:flex" aria-label="Session usage">
					<span title="Input tokens across all replies, including regenerated and tool turns">{fmtNum(totals.input_tokens)} in</span>
					<span>{fmtNum(totals.output_tokens)} out</span>
					{#if totals.cached_tokens != null}<span>{fmtNum(totals.cached_tokens)} cached</span>{/if}
					{#if totals.reasoning_tokens != null}<span>{fmtNum(totals.reasoning_tokens)} reasoning</span>{/if}
					{#if cacheRate != null}<span title="Cached input tokens ÷ all input tokens">cache {cacheRate.toFixed(0)}%</span>{/if}
					{#if totals.cost != null}<span>{fmtCost(totals.cost)}</span>{/if}
				</div>
			{/if}
			<details class="relative">
				<summary class="cursor-pointer list-none rounded-lg px-2 py-1 text-sm hover:bg-zinc-100 dark:hover:bg-zinc-800" aria-label="Chat menu">⋯</summary>
				<div class="absolute right-0 z-20 mt-1 w-48 rounded-xl border border-zinc-200 bg-white p-1 text-sm shadow-lg dark:border-zinc-800 dark:bg-zinc-900">
					<button class="block w-full rounded-lg px-3 py-1.5 text-left hover:bg-zinc-100 dark:hover:bg-zinc-800" onclick={openSettings}>Chat settings</button>
					<a class="block rounded-lg px-3 py-1.5 hover:bg-zinc-100 dark:hover:bg-zinc-800" href="/api/chats/{chatId}/export?format=md" download>Export Markdown</a>
					<a class="block rounded-lg px-3 py-1.5 hover:bg-zinc-100 dark:hover:bg-zinc-800" href="/api/chats/{chatId}/export?format=json" download>Export JSON</a>
				</div>
			</details>
		{:else}
			<span class="flex-1"></span>
		{/if}
	</header>

	<div class="min-h-0 flex-1 overflow-y-auto" bind:this={scroller} {onscroll}>
		<div class="mx-auto max-w-3xl space-y-6 px-4 py-6">
			{#if !chatId}
				<div class="pt-[15vh] text-center">
					<h2 class="text-2xl font-semibold">How can I help?</h2>
					{#if store.modelsLoaded && !store.models.length}
						<p class="muted mt-2">You don't have access to any models yet. {session.me?.user.role === 'admin' ? 'Add a provider under Admin → Providers.' : 'Ask an admin to grant you access.'}</p>
					{/if}
				</div>
			{/if}
			{#each messages as m, i (m.id ?? `draft-${i}`)}
				<div style="content-visibility: auto; contain-intrinsic-size: auto 120px">
					<Message
						{m}
						{busy}
						{showMetrics}
						isLast={i === messages.length - 1}
						onedit={(content) => run({ content, attachments: m.attachments, edit_message_id: m.id })}
						onregenerate={() => run({ regenerate: true })}
					/>
				</div>
			{/each}
			{#if error}<p class="error" role="alert">{error}</p>{/if}
		</div>
	</div>

	<div class="mx-auto w-full max-w-3xl px-4 pb-4">
		<Composer {busy} vision={!!model?.supports_vision} disabled={!modelId} onsend={send} onstop={() => controller?.abort()}>
			{#snippet toolbar()}
				{#if webSearchAvailable && chat}
					<button
						type="button"
						class="rounded-lg px-2 py-1 text-sm {webSearchOn ? 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200' : 'text-zinc-600 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800'}"
						aria-pressed={webSearchOn}
						disabled={model?.forced_skills.includes('web_search')}
						onclick={toggleWebSearch}>🌐 Web search</button
					>
				{/if}
			{/snippet}
		</Composer>
	</div>
</div>

<Modal bind:open={settingsOpen} title="Chat settings">
	<form class="space-y-4" onsubmit={saveSettings}>
		<div>
			<label class="label" for="cs-sys">System prompt</label>
			<textarea id="cs-sys" class="input" rows="4" bind:value={cs.system_prompt} placeholder={model ? 'Uses the model default when empty' : ''}></textarea>
		</div>
		<div class="grid grid-cols-2 gap-4">
			<div><label class="label" for="cs-temp">Temperature</label><input id="cs-temp" class="input" type="number" min="0" max="2" step="0.1" bind:value={cs.temperature} placeholder="Default" /></div>
			<div><label class="label" for="cs-max">Max tokens</label><input id="cs-max" class="input" type="number" min="1" bind:value={cs.max_tokens} placeholder="Default" /></div>
		</div>
		<label class="flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={showMetrics} /> Show speed and token metrics under replies</label>
		<div class="flex justify-end"><button class="btn">Save</button></div>
	</form>
</Modal>
