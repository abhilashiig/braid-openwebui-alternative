<script lang="ts">
	import { get, errorMessage } from '#lib/api.ts';
	import { fmtDate } from '#lib/format.ts';

	let rows = $state<any[]>([]);
	let action = $state('');
	let error = $state('');
	let more = $state(true);

	async function load(reset = true) {
		const before = reset ? '' : `&before=${rows.at(-1)?.id ?? ''}`;
		try {
			const page = await get(`/api/admin/audit?action=${encodeURIComponent(action)}${before}`);
			rows = reset ? page : [...rows, ...page];
			more = page.length === 100;
		} catch (e) {
			error = errorMessage(e);
		}
	}
	$effect(() => {
		action;
		load();
	});
</script>

<div class="flex items-center gap-2">
	<select class="input w-auto" bind:value={action} aria-label="Filter by area">
		<option value="">All actions</option>
		{#each ['provider', 'provider_key', 'model', 'grant', 'group', 'user', 'invitation', 'settings', 'skill', 'api_key'] as a}<option value={a + '.'}>{a}</option>{/each}
	</select>
</div>
{#if error}<p class="error mt-4">{error}</p>{/if}
<div class="mt-4 overflow-x-auto rounded-xl border border-zinc-200 dark:border-zinc-800">
	<table class="w-full text-sm">
		<thead class="bg-zinc-50 text-left text-xs text-zinc-500 uppercase dark:bg-zinc-900">
			<tr><th class="px-4 py-2">When</th><th class="px-4 py-2">Who</th><th class="px-4 py-2">Action</th><th class="px-4 py-2">Details</th><th class="px-4 py-2">From</th></tr>
		</thead>
		<tbody class="divide-y divide-zinc-100 align-top dark:divide-zinc-800">
			{#each rows as r (r.id)}
				<tr>
					<td class="px-4 py-2 whitespace-nowrap text-zinc-500">{fmtDate(r.created_at)}</td>
					<td class="px-4 py-2">{r.actor_email ?? 'system'}</td>
					<td class="px-4 py-2"><code>{r.action}</code></td>
					<td class="max-w-md px-4 py-2"><code class="text-xs break-all text-zinc-600 dark:text-zinc-400">{JSON.stringify(r.details)}</code></td>
					<td class="px-4 py-2 text-zinc-500">{r.ip ?? ''}</td>
				</tr>
			{/each}
		</tbody>
	</table>
</div>
{#if more && rows.length}<button class="btn-secondary mt-4" onclick={() => load(false)}>Load more</button>{/if}
