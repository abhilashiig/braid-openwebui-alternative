<script lang="ts">
	import { store } from '#lib/chats.svelte.ts';

	let { value = $bindable() }: { value: string | null } = $props();
	let open = $state(false);
	let q = $state('');
	let root: HTMLDivElement;

	const current = $derived(store.models.find((m) => m.id === value));
	const filtered = $derived(store.models.filter((m) => `${m.display_name} ${m.provider_name} ${m.name}`.toLowerCase().includes(q.toLowerCase())));

	function pick(id: string) {
		value = id;
		open = false;
		q = '';
	}

	function onWindowClick(e: MouseEvent) {
		if (open && !root.contains(e.target as Node)) open = false;
	}
</script>

<svelte:window onclick={onWindowClick} />

<div class="relative" bind:this={root}>
	<button
		type="button"
		class="flex items-center gap-2 rounded-lg px-2 py-1.5 text-sm font-medium hover:bg-zinc-100 dark:hover:bg-zinc-800"
		aria-haspopup="listbox"
		aria-expanded={open}
		onclick={() => (open = !open)}
	>
		{current ? current.display_name : store.models.length ? 'Choose a model' : 'No models available'}
		{#if current}<span class="text-xs font-normal text-zinc-500">{current.provider_name}</span>{/if}
		<span aria-hidden="true" class="text-xs">▾</span>
	</button>
	{#if open}
		<div class="absolute top-full left-0 z-20 mt-1 w-80 rounded-xl border border-zinc-200 bg-white p-2 shadow-lg dark:border-zinc-800 dark:bg-zinc-900">
			<!-- svelte-ignore a11y_autofocus -->
			<input class="input mb-2" placeholder="Search models" bind:value={q} autofocus aria-label="Search models" onkeydown={(e) => e.key === 'Escape' && (open = false)} />
			<ul role="listbox" class="max-h-80 overflow-y-auto">
				{#each filtered as m (m.id)}
					<li role="option" aria-selected={m.id === value}>
						<button type="button" class="w-full rounded-lg px-2 py-1.5 text-left hover:bg-zinc-100 dark:hover:bg-zinc-800 {m.id === value ? 'bg-zinc-100 dark:bg-zinc-800' : ''}" onclick={() => pick(m.id)}>
							<div class="flex items-center gap-2 text-sm">
								<span class="font-medium">{m.display_name}</span>
								<span class="text-xs text-zinc-500">{m.provider_name}</span>
								<span class="flex-1"></span>
								{#if m.supports_vision}<span class="text-xs text-zinc-400" title="Vision">👁</span>{/if}
								{#if m.supports_tools}<span class="text-xs text-zinc-400" title="Tools">🛠</span>{/if}
								{#if m.supports_reasoning}<span class="text-xs text-zinc-400" title="Reasoning">💭</span>{/if}
							</div>
							{#if m.description}<div class="truncate text-xs text-zinc-500">{m.description}</div>{/if}
						</button>
					</li>
				{:else}
					<li class="muted px-2 py-1.5">No models. Ask an admin for access.</li>
				{/each}
			</ul>
		</div>
	{/if}
</div>
