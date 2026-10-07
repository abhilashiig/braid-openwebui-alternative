<script lang="ts">
	import { del, get, post, errorMessage } from '#lib/api.ts';

	let { providerId, ondone }: { providerId: string; ondone: () => void } = $props();

	type Row = { upstream_id: string; model_id: string | null };
	let available = $state<Row[]>([]);
	let removed = $state<{ model_id: string; upstream_id: string }[]>([]);
	let selected = $state<Record<string, boolean>>({});
	let q = $state('');
	let visibility = $state<'private' | 'public'>('private');
	let error = $state('');
	let loading = $state(true);
	let busy = $state(false);

	const filtered = $derived(available.filter((r) => r.upstream_id.toLowerCase().includes(q.toLowerCase())));
	const fresh = $derived(filtered.filter((r) => !r.model_id));
	const count = $derived(Object.values(selected).filter(Boolean).length);

	get(`/api/admin/providers/${providerId}/upstream-models`)
		.then((r) => {
			available = r.available;
			removed = r.removed;
		})
		.catch((e) => (error = errorMessage(e)))
		.finally(() => (loading = false));

	function toggleAll(on: boolean) {
		for (const r of fresh) selected[r.upstream_id] = on;
	}

	async function importSelected() {
		busy = true;
		error = '';
		try {
			const upstream_ids = Object.keys(selected).filter((k) => selected[k]);
			const res = await post('/api/admin/models/import', { provider_id: providerId, upstream_ids, visibility });
			if (res.errors.length) error = res.errors.map((e: any) => `${e.upstream_id}: ${e.error}`).join('; ');
			else ondone();
		} catch (e) {
			error = errorMessage(e);
		} finally {
			busy = false;
		}
	}

	async function removeModel(id: string, name: string) {
		if (!confirm(`Delete model ${name}? Its grants are removed too.`)) return;
		await del(`/api/admin/models/${id}`);
		removed = removed.filter((r) => r.model_id !== id);
	}
</script>

{#if loading}
	<p class="muted">Fetching models from the provider…</p>
{:else}
	{#if error}<p class="error mb-3" role="alert">{error}</p>{/if}
	{#if removed.length}
		<div class="mb-4 rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm dark:border-amber-800 dark:bg-amber-950">
			<p class="font-medium">No longer offered upstream:</p>
			<ul class="mt-2 space-y-1">
				{#each removed as r}
					<li class="flex items-center justify-between">
						<code>{r.upstream_id}</code>
						<button class="text-red-600 hover:underline" onclick={() => removeModel(r.model_id, r.upstream_id)}>Delete</button>
					</li>
				{/each}
			</ul>
		</div>
	{/if}
	<div class="flex flex-wrap items-center gap-2">
		<input class="input max-w-xs flex-1" placeholder="Search models" bind:value={q} aria-label="Search models" />
		<button class="btn-secondary" onclick={() => toggleAll(true)}>Select all</button>
		<button class="btn-secondary" onclick={() => toggleAll(false)}>None</button>
	</div>
	<ul class="mt-3 max-h-80 divide-y divide-zinc-100 overflow-y-auto rounded-lg border border-zinc-200 dark:divide-zinc-800 dark:border-zinc-800">
		{#each filtered as r (r.upstream_id)}
			<li>
				<label class="flex items-center gap-3 px-3 py-2 text-sm {r.model_id ? 'opacity-50' : 'cursor-pointer hover:bg-zinc-50 dark:hover:bg-zinc-800'}">
					<input type="checkbox" disabled={!!r.model_id} bind:checked={selected[r.upstream_id]} />
					<code class="flex-1 truncate">{r.upstream_id}</code>
					{#if r.model_id}<span class="text-xs">added</span>{/if}
				</label>
			</li>
		{:else}
			<li class="muted px-3 py-2">No models found.</li>
		{/each}
	</ul>
	<fieldset class="mt-4 flex gap-4 text-sm">
		<legend class="label">Visibility for imported models</legend>
		<label class="flex items-center gap-2"><input type="radio" value="private" bind:group={visibility} /> Private (only granted users/groups)</label>
		<label class="flex items-center gap-2"><input type="radio" value="public" bind:group={visibility} /> Public (all users)</label>
	</fieldset>
	<div class="mt-4 flex justify-end">
		<button class="btn" disabled={!count || busy} onclick={importSelected}>Import {count} model{count === 1 ? '' : 's'}</button>
	</div>
{/if}
