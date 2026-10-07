<script lang="ts">
	let {
		busy = false,
		vision = false,
		disabled = false,
		onsend,
		onstop,
		toolbar
	}: {
		busy?: boolean;
		vision?: boolean;
		disabled?: boolean;
		onsend: (content: string, attachments: any[]) => void;
		onstop: () => void;
		toolbar?: import('svelte').Snippet;
	} = $props();

	let text = $state('');
	let attachments = $state<any[]>([]);
	let error = $state('');
	let ta: HTMLTextAreaElement;

	const MAX_IMAGE = 5_000_000;
	const MAX_TEXT = 500_000;

	$effect(() => {
		text;
		ta.style.height = 'auto';
		ta.style.height = Math.min(ta.scrollHeight, 300) + 'px';
	});

	async function addFiles(files: FileList | File[]) {
		error = '';
		for (const f of files) {
			if (f.type.startsWith('image/')) {
				if (!vision) { error = 'The selected model cannot read images.'; continue; }
				if (f.size > MAX_IMAGE) { error = `${f.name} is over 5 MB.`; continue; }
				const url = await new Promise<string>((res) => { const r = new FileReader(); r.onload = () => res(r.result as string); r.readAsDataURL(f); });
				attachments.push({ type: 'image', name: f.name, url });
			} else if (f.type === 'application/pdf' || f.name.toLowerCase().endsWith('.pdf')) {
				error = 'PDF attachments are not supported yet. Paste the text instead.';
			} else {
				if (f.size > MAX_TEXT) { error = `${f.name} is over 500 KB.`; continue; }
				const content = await f.text();
				if (content.includes('\u0000')) { error = `${f.name} is not a text file.`; continue; }
				attachments.push({ type: 'file', name: f.name, text: content });
			}
		}
	}

	function submit() {
		if (busy || disabled || (!text.trim() && !attachments.length)) return;
		onsend(text, attachments);
		text = '';
		attachments = [];
	}

	function onpaste(e: ClipboardEvent) {
		const files = [...(e.clipboardData?.files ?? [])];
		if (files.length) { e.preventDefault(); addFiles(files); }
	}
</script>

<div
	class="rounded-2xl border border-zinc-300 bg-white p-2 shadow-sm focus-within:border-zinc-500 dark:border-zinc-700 dark:bg-zinc-900"
	role="group"
	ondragover={(e) => e.preventDefault()}
	ondrop={(e) => { e.preventDefault(); if (e.dataTransfer?.files) addFiles(e.dataTransfer.files); }}
>
	{#if attachments.length}
		<div class="mb-2 flex flex-wrap gap-2 px-1">
			{#each attachments as a, i}
				<span class="flex items-center gap-1 rounded-lg border border-zinc-200 px-2 py-1 text-xs dark:border-zinc-700">
					{#if a.type === 'image'}<img src={a.url} alt="" class="h-6 w-6 rounded object-cover" />{:else}📄{/if}
					<span class="max-w-40 truncate">{a.name}</span>
					<button type="button" aria-label="Remove {a.name}" onclick={() => attachments.splice(i, 1)}>✕</button>
				</span>
			{/each}
		</div>
	{/if}
	{#if error}<p class="px-2 pb-1 text-xs text-red-600" role="alert">{error}</p>{/if}
	<textarea
		bind:this={ta}
		bind:value={text}
		rows="1"
		class="block w-full resize-none bg-transparent px-2 py-1.5 outline-none"
		placeholder={disabled ? 'Choose a model to start' : 'Message'}
		aria-label="Message"
		{disabled}
		{onpaste}
		onkeydown={(e) => {
			if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); submit(); }
		}}
	></textarea>
	<div class="mt-1 flex items-center gap-2">
		<label class="cursor-pointer rounded-lg px-2 py-1 text-sm text-zinc-600 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800" title="Attach images or text files">
			📎<span class="sr-only">Attach files</span>
			<input type="file" multiple class="sr-only" accept={vision ? 'image/*,text/*,.md,.csv,.json,.py,.js,.ts,.rs,.go,.java,.c,.cpp,.h,.sql,.yaml,.yml,.toml,.xml,.html,.css' : 'text/*,.md,.csv,.json,.py,.js,.ts,.rs,.go,.java,.c,.cpp,.h,.sql,.yaml,.yml,.toml,.xml,.html,.css'} onchange={(e) => { const f = e.currentTarget.files; if (f) addFiles(f); e.currentTarget.value = ''; }} />
		</label>
		{@render toolbar?.()}
		<span class="flex-1"></span>
		{#if busy}
			<button type="button" class="btn" onclick={onstop} aria-label="Stop generating">■ Stop</button>
		{:else}
			<button type="button" class="btn" onclick={submit} disabled={disabled || (!text.trim() && !attachments.length)} aria-label="Send">Send ↵</button>
		{/if}
	</div>
</div>
