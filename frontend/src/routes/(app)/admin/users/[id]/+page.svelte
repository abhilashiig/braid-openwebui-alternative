<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { del, get, patch, post, put, errorMessage } from '#lib/api.ts';
	import { session } from '#lib/session.svelte.ts';
	import { timeAgo } from '#lib/format.ts';
	import MultiPick from '#lib/components/MultiPick.svelte';
	import CopyButton from '#lib/components/CopyButton.svelte';
	import Modal from '#lib/components/Modal.svelte';

	const id = page.params.id!;
	let data = $state<any>(null);
	let groups = $state<any[]>([]);
	let memberOf = $state<string[]>([]);
	let error = $state('');
	let notice = $state('');
	let resetLink = $state('');
	let deleteOpen = $state(false);
	let eraseChats = $state(false);
	let grantModels = $state<string[]>([]);

	async function load() {
		try {
			data = await get(`/api/admin/users/${id}`);
			memberOf = data.user.group_ids;
		} catch (e) {
			error = errorMessage(e);
		}
	}
	load();
	get('/api/admin/groups').then((g) => (groups = g.filter((x: any) => !x.is_everyone)));

	async function update(body: any, msg: string) {
		error = notice = '';
		try {
			await patch(`/api/admin/users/${id}`, body);
			notice = msg;
			load();
		} catch (e) {
			error = errorMessage(e);
		}
	}

	async function saveGroups() {
		await put(`/api/admin/users/${id}/groups`, { group_ids: memberOf });
		notice = 'Groups saved';
		load();
	}

	async function forceReset() {
		if (!confirm('Sign this user out everywhere and create a password reset link?')) return;
		const r = await post(`/api/admin/users/${id}/reset-link`);
		resetLink = r.link;
		if (r.emailed) notice = 'Reset link emailed to the user';
	}

	async function grantDirect() {
		for (const m of grantModels) {
			const g = await get(`/api/admin/models/${m}/grants`);
			await put(`/api/admin/models/${m}/grants`, { user_ids: [...g.users.map((u: any) => u.id), id], group_ids: g.groups.map((x: any) => x.id) });
		}
		grantModels = [];
		load();
	}

	async function revokeDirect(grantId: string) {
		await del(`/api/admin/grants/${grantId}`);
		load();
	}

	async function remove() {
		try {
			await del(`/api/admin/users/${id}?erase_chats=${eraseChats}`);
			goto('/admin/users');
		} catch (e) {
			error = errorMessage(e);
			deleteOpen = false;
		}
	}
</script>

<a href="/admin/users" class="text-sm text-zinc-500 hover:underline">← Users</a>
{#if error}<p class="error mt-4" role="alert">{error}</p>{/if}
{#if notice}<p class="mt-4 text-sm text-green-700 dark:text-green-400" role="status">{notice}</p>{/if}

{#if data}
	{@const u = data.user}
	<div class="mt-4 flex flex-wrap items-start justify-between gap-4">
		<div>
			<h2 class="text-xl font-semibold">{u.name}</h2>
			<p class="muted">{u.email} · {u.role} · {u.status} · last active {timeAgo(u.last_active_at)}</p>
		</div>
		<div class="flex flex-wrap gap-2">
			{#if u.role === 'user'}
				<button class="btn-secondary" onclick={() => update({ role: 'admin' }, 'Promoted to admin')}>Make admin</button>
			{:else}
				<button class="btn-secondary" onclick={() => update({ role: 'user' }, 'Changed to user')}>Remove admin</button>
			{/if}
			{#if u.status === 'active'}
				<button class="btn-secondary" onclick={() => update({ status: 'deactivated' }, 'Deactivated: sessions ended, API keys stopped')}>Deactivate</button>
			{:else}
				<button class="btn-secondary" onclick={() => update({ status: 'active' }, 'Reactivated')}>Reactivate</button>
			{/if}
			<button class="btn-secondary" onclick={forceReset}>Force password reset</button>
			{#if u.id !== session.me?.user.id}<button class="btn-danger" onclick={() => (deleteOpen = true)}>Delete</button>{/if}
		</div>
	</div>
	{#if resetLink}
		<div class="card mt-4 flex items-center gap-2 text-sm">
			<span>Reset link (valid 24 h):</span><code class="min-w-0 flex-1 truncate">{resetLink}</code><CopyButton text={resetLink} />
		</div>
	{/if}

	<div class="mt-6 grid gap-6 lg:grid-cols-2">
		<section class="card">
			<h3 class="font-semibold">Groups</h3>
			<p class="muted">Everyone is implicit for all active users.</p>
			<div class="mt-3"><MultiPick items={groups.map((g) => ({ id: g.id, label: g.name }))} bind:selected={memberOf} placeholder="Add group" /></div>
			<button class="btn mt-3" onclick={saveGroups}>Save groups</button>
		</section>
		<section class="card">
			<h3 class="font-semibold">Platform API keys</h3>
			<label class="mt-3 block text-sm" for="api-keys">Allow this user to create API keys</label>
			<select
				id="api-keys"
				class="input mt-1"
				value={u.allow_api_keys === null ? 'inherit' : String(u.allow_api_keys)}
				onchange={(e) => {
					const v = e.currentTarget.value;
					update({ allow_api_keys: v === 'inherit' ? null : v === 'true' }, 'API key permission saved');
				}}
			>
				<option value="inherit">Inherit from groups / global setting</option>
				<option value="true">Allow</option>
				<option value="false">Deny</option>
			</select>
		</section>
	</div>

	<section class="card mt-6">
		<h3 class="font-semibold">Effective model access</h3>
		<p class="muted">Each model and why this user can use it.</p>
		<ul class="mt-3 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
			{#each data.models as m (m.id)}
				<li class="flex flex-wrap items-center gap-3 py-2 {m.allowed ? '' : 'opacity-50'}">
					<span class="font-medium">{m.display_name}</span>
					<span class="text-xs text-zinc-500">{m.provider_name}</span>
					<span class="flex-1"></span>
					{#if !m.allowed}<span class="text-xs">no access</span>{/if}
					{#if m.is_public}<span class="rounded bg-green-100 px-1.5 text-xs text-green-800 dark:bg-green-900 dark:text-green-200">public</span>{/if}
					{#if m.direct}
						<span class="rounded bg-blue-100 px-1.5 text-xs text-blue-800 dark:bg-blue-900 dark:text-blue-200">direct grant</span>
						<button class="text-xs text-red-600 hover:underline" onclick={() => revokeDirect(m.direct_grant_id)}>Revoke</button>
					{/if}
					{#each m.via_groups as g}<span class="rounded bg-zinc-100 px-1.5 text-xs dark:bg-zinc-800">via {g}</span>{/each}
					{#if !m.enabled}<span class="text-xs text-amber-700">disabled</span>{/if}
				</li>
			{/each}
		</ul>
		<div class="mt-4">
			<span class="label">Grant models directly</span>
			<MultiPick items={data.models.filter((m: any) => !m.allowed).map((m: any) => ({ id: m.id, label: m.display_name, hint: m.provider_name }))} bind:selected={grantModels} placeholder="Add private model" />
			<button class="btn mt-2" disabled={!grantModels.length} onclick={grantDirect}>Grant</button>
		</div>
	</section>

	<Modal bind:open={deleteOpen} title="Delete {u.name}?">
		<div class="space-y-3 text-sm">
			<label class="flex items-center gap-2"><input type="radio" name="erase" value={false} bind:group={eraseChats} /> Keep their chats (account is anonymized)</label>
			<label class="flex items-center gap-2"><input type="radio" name="erase" value={true} bind:group={eraseChats} /> Erase their chats permanently</label>
			<div class="flex justify-end gap-2 pt-2">
				<button class="btn-secondary" onclick={() => (deleteOpen = false)}>Cancel</button>
				<button class="btn-danger" onclick={remove}>Delete user</button>
			</div>
		</div>
	</Modal>
{/if}
