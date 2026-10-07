<script lang="ts">
	import { get, errorMessage } from '#lib/api.ts';
	import { fmtCost, fmtNum } from '#lib/format.ts';

	let by = $state('day');
	let days = $state(30);
	let source = $state('');
	let data = $state<any>(null);
	let error = $state('');

	$effect(() => {
		const q = `by=${by}&days=${days}${source ? `&source=${source}` : ''}`;
		get(`/api/admin/usage?${q}`).then((d) => (data = d)).catch((e) => (error = errorMessage(e)));
	});

	const totals = $derived(
		data?.rows.reduce(
			(a: any, r: any) => ({ requests: a.requests + r.requests, input: a.input + (r.input_tokens ?? 0), output: a.output + (r.output_tokens ?? 0), cost: r.cost == null ? a.cost : (a.cost ?? 0) + r.cost, errors: a.errors + r.errors }),
			{ requests: 0, input: 0, output: 0, cost: null as number | null, errors: 0 }
		)
	);
	const tabs = [['day', 'By day'], ['user', 'By user'], ['model', 'By model'], ['provider', 'By provider'], ['key', 'By API key']];
</script>

<div class="flex flex-wrap items-center gap-2">
	<div class="flex rounded-lg border border-zinc-200 p-0.5 dark:border-zinc-800" role="tablist">
		{#each tabs as [k, label]}
			<button role="tab" aria-selected={by === k} class="rounded-md px-3 py-1 text-sm {by === k ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900' : ''}" onclick={() => (by = k)}>{label}</button>
		{/each}
	</div>
	<select class="input w-auto" bind:value={source} aria-label="Source"><option value="">Chat + API</option><option value="chat">Chat only</option><option value="api">API only</option></select>
	<select class="input w-auto" bind:value={days} aria-label="Period"><option value={1}>Last 24 hours</option><option value={7}>Last 7 days</option><option value={30}>Last 30 days</option><option value={90}>Last 90 days</option></select>
	<span class="flex-1"></span>
	<a class="btn-secondary" href="/api/admin/usage.csv?days={days}" download>Export CSV</a>
</div>
{#if error}<p class="error mt-4">{error}</p>{/if}

{#if data}
	<div class="mt-4 grid gap-4 sm:grid-cols-5">
		{#each [['Requests', fmtNum(totals.requests)], ['Errors', fmtNum(totals.errors)], ['Input tokens', fmtNum(totals.input)], ['Output tokens', fmtNum(totals.output)], ['Est. cost', fmtCost(totals.cost)]] as [label, value]}
			<div class="card p-4"><div class="text-xs text-zinc-500">{label}</div><div class="mt-1 text-xl font-semibold">{value}</div></div>
		{/each}
	</div>
	<p class="muted mt-2">Web searches in this period: {data.web_searches}</p>
	<div class="mt-4 overflow-x-auto rounded-xl border border-zinc-200 dark:border-zinc-800">
		<table class="w-full text-sm">
			<thead class="bg-zinc-50 text-left text-xs text-zinc-500 uppercase dark:bg-zinc-900">
				<tr>
					<th class="px-3 py-2">{tabs.find((t) => t[0] === by)?.[1].replace('By ', '')}</th>
					<th class="px-3 py-2 text-right">Requests</th><th class="px-3 py-2 text-right">Errors</th>
					<th class="px-3 py-2 text-right">In</th><th class="px-3 py-2 text-right">Out</th><th class="px-3 py-2 text-right">Cached</th>
					<th class="px-3 py-2 text-right">Cache rate</th><th class="px-3 py-2 text-right">Avg tok/s</th><th class="px-3 py-2 text-right">Avg TTFT</th><th class="px-3 py-2 text-right">Cost</th>
				</tr>
			</thead>
			<tbody class="divide-y divide-zinc-100 dark:divide-zinc-800">
				{#each data.rows as r}
					<tr>
						<td class="px-3 py-2">{r.label}</td>
						<td class="px-3 py-2 text-right">{fmtNum(r.requests)}</td>
						<td class="px-3 py-2 text-right {r.errors ? 'text-red-600' : ''}">{r.errors}</td>
						<td class="px-3 py-2 text-right">{fmtNum(r.input_tokens)}</td>
						<td class="px-3 py-2 text-right">{fmtNum(r.output_tokens)}</td>
						<td class="px-3 py-2 text-right">{fmtNum(r.cached_tokens)}</td>
						<td class="px-3 py-2 text-right">{r.cache_rate == null ? '–' : `${(r.cache_rate * 100).toFixed(0)}%`}</td>
						<td class="px-3 py-2 text-right">{r.avg_output_tps == null ? '–' : r.avg_output_tps.toFixed(0)}</td>
						<td class="px-3 py-2 text-right">{r.avg_ttft_ms == null ? '–' : `${r.avg_ttft_ms.toFixed(0)} ms`}</td>
						<td class="px-3 py-2 text-right">{fmtCost(r.cost)}</td>
					</tr>
				{:else}
					<tr><td colspan="10" class="muted px-3 py-6 text-center">No usage in this period.</td></tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
