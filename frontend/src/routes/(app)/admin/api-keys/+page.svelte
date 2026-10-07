<script lang="ts">
	import { del, get, errorMessage } from '#lib/api.ts';
	import { timeAgo } from '#lib/format.ts';

	let keys = $state<any[]>([]);
	let error = $state('');
	const load = () => get('/api/admin/api-keys').then((k) => (keys = k)).catch((e) => (error = errorMessage(e)));
	load();

	async function revoke(k: any) {
		if (!confirm(`Revoke ${k.user_name}'s key “${k.name}”?`)) return;
		await del(`/api/admin/api-keys/${k.id}`);
		load();
	}
</script>

<p class="muted">Platform API keys created by users. Enable API keys under Settings (globally), on a group, or on a user.</p>
{#if error}<p class="error mt-4">{error}</p>{/if}
<div class="mt-4 overflow-x-auto rounded-xl border border-zinc-200 dark:border-zinc-800">
	<table class="w-full text-sm">
		<thead class="bg-zinc-50 text-left text-xs text-zinc-500 uppercase dark:bg-zinc-900">
			<tr><th class="px-4 py-2">Key</th><th class="px-4 py-2">Owner</th><th class="px-4 py-2">Last used</th><th class="px-4 py-2">Status</th><th class="px-4 py-2"></th></tr>
		</thead>
		<tbody class="divide-y divide-zinc-100 dark:divide-zinc-800">
			{#each keys as k (k.id)}
				<tr class={k.revoked_at ? 'opacity-50' : ''}>
					<td class="px-4 py-2"><div class="font-medium">{k.name}</div><code class="text-xs">{k.prefix}…</code></td>
					<td class="px-4 py-2">{k.user_name}<div class="text-xs text-zinc-500">{k.user_email}</div></td>
					<td class="px-4 py-2 text-zinc-500">{timeAgo(k.last_used_at)}{k.last_used_ip ? ` · ${k.last_used_ip}` : ''}</td>
					<td class="px-4 py-2">{k.revoked_at ? 'revoked' : k.expires_at && new Date(k.expires_at) < new Date() ? 'expired' : 'active'}</td>
					<td class="px-4 py-2 text-right">{#if !k.revoked_at}<button class="text-xs text-red-600 hover:underline" onclick={() => revoke(k)}>Revoke</button>{/if}</td>
				</tr>
			{:else}
				<tr><td colspan="5" class="muted px-4 py-6 text-center">No API keys yet.</td></tr>
			{/each}
		</tbody>
	</table>
</div>
