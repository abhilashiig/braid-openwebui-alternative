<script lang="ts">
	import { renderMarkdown, renderState } from '#lib/markdown.svelte.ts';

	let { source }: { source: string } = $props();
	const html = $derived((renderState.version, renderMarkdown(source)));

	function onclick(e: MouseEvent) {
		const btn = (e.target as HTMLElement).closest('[data-copy]') as HTMLButtonElement | null;
		if (!btn) return;
		const code = btn.closest('.code-block')?.querySelector('code')?.textContent ?? '';
		navigator.clipboard.writeText(code);
		btn.textContent = 'Copied';
		setTimeout(() => (btn.textContent = 'Copy'), 1500);
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="prose prose-zinc max-w-none dark:prose-invert prose-pre:my-0 prose-pre:bg-transparent prose-pre:p-0" {onclick}>
	{@html html}
</div>
