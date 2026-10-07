<script lang="ts">
	import type { Snippet } from 'svelte';
	import { session } from '#lib/session.svelte';

	let { title, subtitle, children }: { title: string; subtitle?: string; children: Snippet } = $props();
</script>

<main class="grid min-h-dvh place-items-center bg-zinc-50 px-4 py-10 dark:bg-zinc-950">
	<div class="w-full max-w-md">
		<div class="mb-6 flex items-center gap-2 text-lg font-semibold">
			{#if session.instance?.logo_url}
				<img src={session.instance.logo_url} alt="" class="h-7 w-7 rounded" />
			{/if}
			{session.instance?.name ?? 'Braid'}
		</div>
		<div class="card">
			<h1 class="text-xl font-semibold">{title}</h1>
			{#if subtitle}<p class="muted mt-1">{subtitle}</p>{/if}
			<div class="mt-6">{@render children()}</div>
		</div>
	</div>
</main>
