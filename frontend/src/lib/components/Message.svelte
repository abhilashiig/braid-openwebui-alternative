<script lang="ts">
	import Markdown from './Markdown.svelte';
	import CopyButton from './CopyButton.svelte';
	import { fmtNum, fmtCost } from '#lib/format.ts';

	let {
		m,
		isLast = false,
		busy = false,
		showMetrics = true,
		onedit,
		onregenerate
	}: {
		m: any;
		isLast?: boolean;
		busy?: boolean;
		showMetrics?: boolean;
		onedit?: (content: string) => void;
		onregenerate?: () => void;
	} = $props();

	let editing = $state(false);
	let draft = $state('');

	const metrics = $derived(m.metrics);
	const measured = $derived(metrics?.source !== 'provider');
	const speedTip = $derived(measured ? 'Measured by Braid from stream timing; includes network latency' : 'Reported by the provider');
	const tps = (n: number | null | undefined) => (n == null ? null : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : n.toFixed(n < 10 ? 1 : 0));
	const images = $derived((m.attachments ?? []).filter((a: any) => a.type === 'image'));
	const files = $derived((m.attachments ?? []).filter((a: any) => a.type === 'file'));

	function save() {
		editing = false;
		if (draft.trim() && draft !== m.content) onedit?.(draft);
	}
</script>

{#if m.role === 'user'}
	<div class="group flex flex-col items-end">
		{#if images.length || files.length}
			<div class="mb-1 flex flex-wrap justify-end gap-2">
				{#each images as a}<img src={a.url} alt={a.name} class="max-h-40 rounded-lg border border-zinc-200 dark:border-zinc-800" />{/each}
				{#each files as a}<span class="rounded-lg border border-zinc-200 px-2 py-1 text-xs dark:border-zinc-800">📄 {a.name}</span>{/each}
			</div>
		{/if}
		{#if editing}
			<div class="w-full max-w-2xl">
				<!-- svelte-ignore a11y_autofocus -->
				<textarea class="input min-h-24" bind:value={draft} autofocus aria-label="Edit message" onkeydown={(e) => {
					if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); save(); }
					if (e.key === 'Escape') editing = false;
				}}></textarea>
				<div class="mt-2 flex justify-end gap-2">
					<button class="btn-secondary" onclick={() => (editing = false)}>Cancel</button>
					<button class="btn" onclick={save}>Send</button>
				</div>
			</div>
		{:else}
			<div class="max-w-[85%] rounded-2xl bg-zinc-100 px-4 py-2 whitespace-pre-wrap dark:bg-zinc-800">{m.content}</div>
			{#if m.id && !busy}
				<div class="mt-1 flex gap-1 opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100">
					<CopyButton text={m.content} />
					<button class="rounded px-2 py-1 text-xs text-zinc-600 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800" onclick={() => ((draft = m.content), (editing = true))}>Edit</button>
				</div>
			{/if}
		{/if}
	</div>
{:else}
	<div class="group">
		<div class="mb-1 text-xs font-medium text-zinc-500">{m.model_name ?? 'Assistant'}</div>
		{#if m.reasoning}
			<details class="mb-2 rounded-lg border border-zinc-200 px-3 py-2 text-sm dark:border-zinc-800" open={m.streaming && !m.content}>
				<summary class="cursor-pointer text-zinc-500">{m.streaming && !m.content ? 'Thinking…' : 'Thought process'}</summary>
				<div class="mt-2 whitespace-pre-wrap text-zinc-600 dark:text-zinc-400">{m.reasoning}</div>
			</details>
		{/if}
		{#each m.tool_steps ?? [] as step (step.id)}
			<details class="mb-2 rounded-lg border border-zinc-200 px-3 py-2 text-sm dark:border-zinc-800">
				<summary class="cursor-pointer text-zinc-600 dark:text-zinc-400">
					{step.pending ? '⏳' : step.error ? '⚠️' : '🔎'} {step.summary ?? `Calling ${step.name}…`}
				</summary>
				{#if step.error}<p class="mt-2 text-red-600">{step.error}</p>{/if}
				{#if step.sources?.length}
					<ol class="mt-2 list-decimal space-y-1 pl-5">
						{#each step.sources as s}
							<li><a class="underline" href={s.url} target="_blank" rel="noopener noreferrer">{s.title || s.url}</a>{#if s.snippet}<div class="text-xs text-zinc-500">{s.snippet}</div>{/if}</li>
						{/each}
					</ol>
				{/if}
			</details>
		{/each}
		{#if m.content}
			<Markdown source={m.content} />
		{:else if m.streaming && !m.reasoning}
			<p class="animate-pulse text-zinc-400" aria-live="polite">…</p>
		{/if}
		{#if m.error}
			<div class="mt-2 {m.error === 'Stopped' ? 'muted' : 'error'}" role={m.error === 'Stopped' ? undefined : 'alert'}>
				{m.error === 'Stopped' ? 'Stopped.' : m.error}
				{#if isLast && !busy && m.error !== 'Stopped'}<button class="ml-2 underline" onclick={onregenerate}>Retry</button>{/if}
			</div>
		{/if}
		{#if m.sources?.length && !m.streaming}
			<div class="mt-3 flex flex-wrap gap-2">
				{#each m.sources as s, i}
					<a href={s.url} target="_blank" rel="noopener noreferrer" class="max-w-xs truncate rounded-full border border-zinc-200 px-2.5 py-1 text-xs hover:bg-zinc-50 dark:border-zinc-800 dark:hover:bg-zinc-900" title={s.url}>
						{i + 1}. {s.title || new URL(s.url).hostname}
					</a>
				{/each}
			</div>
		{/if}
		{#if !m.streaming}
			<div class="mt-1 flex flex-wrap items-center gap-x-1 text-xs text-zinc-500">
				{#if m.content}<CopyButton text={m.content} />{/if}
				{#if isLast && !busy && !m.error}<button class="rounded px-2 py-1 font-medium text-zinc-600 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800" onclick={onregenerate}>Regenerate</button>{/if}
				{#if showMetrics && metrics}
					<span class="ml-1 flex flex-wrap items-center gap-x-2">
						{#if metrics.output_tps != null}<span title={speedTip}>{tps(metrics.output_tps)} tok/s{measured ? '*' : ''}</span>{/if}
						{#if metrics.prefill_tps != null}<span title={speedTip}>prefill {tps(metrics.prefill_tps)} tok/s{measured ? '*' : ''}</span>{/if}
						{#if metrics.ttft_ms != null}<span title="Time to first token, measured by Braid">TTFT {metrics.ttft_ms} ms</span>{/if}
						{#if metrics.input_tokens != null}<span>{fmtNum(metrics.input_tokens)} in</span>{/if}
						{#if metrics.output_tokens != null}<span>{fmtNum(metrics.output_tokens)} out</span>{/if}
						{#if metrics.cached_tokens != null || metrics.cache_write_tokens != null || metrics.cost != null || metrics.reasoning_tokens != null}
							<details class="relative inline-block">
								<summary class="cursor-pointer list-none underline decoration-dotted">details</summary>
								<div class="absolute bottom-full left-0 z-10 mb-1 w-56 rounded-lg border border-zinc-200 bg-white p-2 shadow dark:border-zinc-800 dark:bg-zinc-900">
									{#if metrics.cached_tokens != null}<div>Cached input: {fmtNum(metrics.cached_tokens)}</div>{/if}
									{#if metrics.cache_write_tokens != null}<div>Cache writes: {fmtNum(metrics.cache_write_tokens)}</div>{/if}
									{#if metrics.reasoning_tokens != null}<div>Reasoning tokens: {fmtNum(metrics.reasoning_tokens)}</div>{/if}
									{#if metrics.cost != null}<div>Estimated cost: {fmtCost(metrics.cost)}</div>{/if}
									{#if measured}<div class="mt-1 text-zinc-400">* measured, includes network latency</div>{/if}
								</div>
							</details>
						{/if}
					</span>
				{/if}
			</div>
		{/if}
	</div>
{/if}
