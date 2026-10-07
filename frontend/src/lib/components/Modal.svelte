<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		open = $bindable(false),
		title,
		wide = false,
		children,
		footer
	}: { open: boolean; title: string; wide?: boolean; children: Snippet; footer?: Snippet } = $props();

	let dialog: HTMLDialogElement;

	$effect(() => {
		if (open && !dialog.open) dialog.showModal();
		if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	onclose={() => (open = false)}
	class="m-auto w-[calc(100%-2rem)] {wide ? 'max-w-3xl' : 'max-w-lg'} rounded-xl border border-zinc-200 bg-white p-0 text-zinc-900 shadow-xl backdrop:bg-black/40 dark:border-zinc-800 dark:bg-zinc-900 dark:text-zinc-100"
>
	{#if open}
		<div class="flex items-center justify-between border-b border-zinc-200 px-5 py-3 dark:border-zinc-800">
			<h2 class="font-semibold">{title}</h2>
			<button class="rounded p-1 text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800" aria-label="Close" onclick={() => (open = false)}>✕</button>
		</div>
		<div class="max-h-[70vh] overflow-y-auto px-5 py-4">{@render children()}</div>
		{#if footer}
			<div class="flex justify-end gap-2 border-t border-zinc-200 px-5 py-3 dark:border-zinc-800">{@render footer()}</div>
		{/if}
	{/if}
</dialog>
