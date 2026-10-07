<script lang="ts">
	import { del, get, post, errorMessage } from '#lib/api.ts';
	import { timeAgo } from '#lib/format.ts';
	import Modal from '#lib/components/Modal.svelte';
	import MultiPick from '#lib/components/MultiPick.svelte';
	import CopyButton from '#lib/components/CopyButton.svelte';

	let users = $state<any[]>([]);
	let groups = $state<any[]>([]);
	let invitations = $state<any[]>([]);
	let filter = $state({ q: '', role: '', status: '', group_id: '' });
	let error = $state('');
	let inviteOpen = $state(false);
	let emailsText = $state('');
	let inviteRole = $state('user');
	let inviteGroups = $state<string[]>([]);
	let inviteResult = $state<{ invited: any[]; errors: any[] } | null>(null);
	let links = $state<Record<string, string>>({});

	async function load() {
		const params = new URLSearchParams(Object.entries(filter).filter(([, v]) => v));
		try {
			[users, invitations] = await Promise.all([get(`/api/admin/users?${params}`), get('/api/admin/invitations')]);
		} catch (e) {
			error = errorMessage(e);
		}
	}
	get('/api/admin/groups').then((g) => (groups = g));
	$effect(() => {
		JSON.stringify(filter);
		const t = setTimeout(load, 200);
		return () => clearTimeout(t);
	});

	function parseEmails(text: string) {
		return [...new Set(text.split(/[\s,;]+/).map((s) => s.trim()).filter((s) => s.includes('@')))];
	}

	async function onCsv(e: Event) {
		const file = (e.currentTarget as HTMLInputElement).files?.[0];
		if (file) emailsText += (emailsText ? '\n' : '') + parseEmails(await file.text()).join('\n');
	}

	async function invite(e: SubmitEvent) {
		e.preventDefault();
		try {
			inviteResult = await post('/api/admin/invitations', { emails: parseEmails(emailsText), role: inviteRole, group_ids: inviteGroups });
			emailsText = '';
			load();
		} catch (err) {
			error = errorMessage(err);
		}
	}

	async function resend(i: any) {
		const r = await post(`/api/admin/invitations/${i.id}/resend`);
		links[i.id] = r.link;
	}

	async function revoke(i: any) {
		if (!confirm(`Revoke the invitation for ${i.email}?`)) return;
		await del(`/api/admin/invitations/${i.id}`);
		load();
	}

	function openInvite() {
		inviteResult = null;
		inviteOpen = true;
	}
</script>

<div class="flex flex-wrap items-center gap-2">
	<input class="input max-w-xs" placeholder="Search name or email" bind:value={filter.q} aria-label="Search users" />
	<select class="input w-auto" bind:value={filter.role} aria-label="Role">
		<option value="">All roles</option><option value="admin">Admins</option><option value="user">Users</option>
	</select>
	<select class="input w-auto" bind:value={filter.status} aria-label="Status">
		<option value="">Any status</option><option value="active">Active</option><option value="deactivated">Deactivated</option>
	</select>
	<select class="input w-auto" bind:value={filter.group_id} aria-label="Group">
		<option value="">Any group</option>
		{#each groups.filter((g) => !g.is_everyone) as g}<option value={g.id}>{g.name}</option>{/each}
	</select>
	<span class="flex-1"></span>
	<button class="btn" onclick={openInvite}>Invite users</button>
</div>
{#if error}<p class="error mt-4">{error}</p>{/if}

<div class="mt-4 overflow-x-auto rounded-xl border border-zinc-200 dark:border-zinc-800">
	<table class="w-full text-sm">
		<thead class="bg-zinc-50 text-left text-xs text-zinc-500 uppercase dark:bg-zinc-900">
			<tr><th class="px-4 py-2">User</th><th class="px-4 py-2">Role</th><th class="px-4 py-2">Groups</th><th class="px-4 py-2">Status</th><th class="px-4 py-2">Last active</th></tr>
		</thead>
		<tbody class="divide-y divide-zinc-100 dark:divide-zinc-800">
			{#each users as u (u.id)}
				<tr class="hover:bg-zinc-50 dark:hover:bg-zinc-900">
					<td class="px-4 py-2">
						<a class="font-medium hover:underline" href="/admin/users/{u.id}">{u.name}</a>
						<div class="text-xs text-zinc-500">{u.email}</div>
					</td>
					<td class="px-4 py-2 capitalize">{u.role}</td>
					<td class="px-4 py-2 text-zinc-600 dark:text-zinc-400">{u.groups.join(', ') || '–'}</td>
					<td class="px-4 py-2">
						<span class={u.status === 'active' ? 'text-green-700 dark:text-green-400' : 'text-zinc-500'}>{u.status}</span>
					</td>
					<td class="px-4 py-2 text-zinc-500">{timeAgo(u.last_active_at)}</td>
				</tr>
			{:else}
				<tr><td colspan="5" class="muted px-4 py-6 text-center">No users match.</td></tr>
			{/each}
		</tbody>
	</table>
</div>

{#if invitations.length}
	<h2 class="mt-8 font-semibold">Pending invitations</h2>
	<ul class="mt-2 divide-y divide-zinc-100 rounded-xl border border-zinc-200 text-sm dark:divide-zinc-800 dark:border-zinc-800">
		{#each invitations as i (i.id)}
			<li class="flex flex-wrap items-center gap-3 px-4 py-2">
				<span class="font-medium">{i.email}</span>
				<span class="text-zinc-500 capitalize">{i.role}</span>
				<span class="text-xs text-zinc-500">expires {new Date(i.expires_at).toLocaleDateString()}</span>
				<span class="flex-1"></span>
				{#if links[i.id]}
					<code class="max-w-xs truncate text-xs">{links[i.id]}</code><CopyButton text={links[i.id]} />
				{:else}
					<button class="text-xs hover:underline" onclick={() => resend(i)}>New link</button>
				{/if}
				<button class="text-xs text-red-600 hover:underline" onclick={() => revoke(i)}>Revoke</button>
			</li>
		{/each}
	</ul>
{/if}

<Modal bind:open={inviteOpen} title="Invite users" wide>
	{#if inviteResult}
		<div class="space-y-3">
			{#if inviteResult.invited.length}
				<p class="text-sm">Send each person their link. Links are single-use.</p>
				<ul class="divide-y divide-zinc-100 rounded-lg border border-zinc-200 text-sm dark:divide-zinc-800 dark:border-zinc-800">
					{#each inviteResult.invited as i}
						<li class="flex items-center gap-2 px-3 py-2">
							<span class="w-48 shrink-0 truncate font-medium">{i.email}</span>
							<code class="min-w-0 flex-1 truncate text-xs">{i.link}</code>
							<CopyButton text={i.link} />
						</li>
					{/each}
				</ul>
				<CopyButton text={inviteResult.invited.map((i) => `${i.email}\t${i.link}`).join('\n')} label="Copy all" />
			{/if}
			{#each inviteResult.errors as e}<p class="error">{e.email}: {e.error}</p>{/each}
			<div class="flex justify-end"><button class="btn" onclick={() => (inviteOpen = false)}>Done</button></div>
		</div>
	{:else}
		<form class="space-y-4" onsubmit={invite}>
			<div>
				<label class="label" for="emails">Emails</label>
				<textarea id="emails" class="input" rows="5" bind:value={emailsText} placeholder="one@example.com, two@example.com"></textarea>
				<p class="muted mt-1">{parseEmails(emailsText).length} address(es). Paste a list or <label class="cursor-pointer underline">upload a CSV<input type="file" accept=".csv,text/csv,text/plain" class="sr-only" onchange={onCsv} /></label>.</p>
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<div>
					<label class="label" for="role">Role</label>
					<select id="role" class="input" bind:value={inviteRole}><option value="user">User</option><option value="admin">Admin</option></select>
				</div>
				<div>
					<span class="label">Groups</span>
					<MultiPick items={groups.filter((g) => !g.is_everyone).map((g) => ({ id: g.id, label: g.name }))} bind:selected={inviteGroups} placeholder="Add group" />
				</div>
			</div>
			<div class="flex justify-end"><button class="btn" disabled={!parseEmails(emailsText).length}>Create invitations</button></div>
		</form>
	{/if}
</Modal>
