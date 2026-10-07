<script lang="ts">
	type Item = { id: string; label: string; hint?: string };
	let { items, selected = $bindable([]), placeholder = 'Search' }: { items: Item[]; selected: string[]; placeholder?: string } = $props();
	let q = $state('');
	const chosen = $derived(items.filter((i) => selected.includes(i.id)));
	const matches = $derived(
		q ? items.filter((i) => !selected.includes(i.id) && `${i.label} ${i.hint ?? ''}`.toLowerCase().includes(q.toLowerCase())).slice(0, 8) : []
	);
</script>

<div>
	<div class="flex flex-wrap gap-1">
		{#each chosen as c (c.id)}
			<span class="inline-flex items-center gap-1 rounded-full bg-zinc-100 px-2 py-0.5 text-sm dark:bg-zinc-800">
				{c.label}
				<button type="button" aria-label="Remove {c.label}" class="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100" onclick={() => (selected = selected.filter((s) => s !== c.id))}>✕</button>
			</span>
		{/each}
	</div>
	<input class="input mt-2" {placeholder} bind:value={q} aria-label={placeholder} />
	{#if matches.length}
		<ul class="mt-1 rounded-lg border border-zinc-200 dark:border-zinc-700">
			{#each matches as m (m.id)}
				<li>
					<button type="button" class="w-full px-3 py-1.5 text-left text-sm hover:bg-zinc-50 dark:hover:bg-zinc-800" onclick={() => ((selected = [...selected, m.id]), (q = ''))}>
						{m.label} {#if m.hint}<span class="text-zinc-500">{m.hint}</span>{/if}
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</div>
