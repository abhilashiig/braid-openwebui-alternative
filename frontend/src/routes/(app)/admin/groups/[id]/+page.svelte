<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { del, get, patch, post, errorMessage } from '#lib/api.ts';
	import MultiPick from '#lib/components/MultiPick.svelte';

	const id = page.params.id!;
	let data = $state<any>(null);
	let users = $state<any[]>([]);
	let models = $state<any[]>([]);
	let addUsers = $state<string[]>([]);
	let addModels = $state<string[]>([]);
	let name = $state('');
	let description = $state('');
	let error = $state('');
	let notice = $state('');

	async function load() {
		try {
			data = await get(`/api/admin/groups/${id}`);
			name = data.group.name;
			description = data.group.description;
		} catch (e) {
			error = errorMessage(e);
		}
	}
	load();
	get('/api/admin/users').then((u) => (users = u));
	get('/api/admin/models').then((m) => (models = m));

	async function run(fn: () => Promise<any>, msg = '') {
		error = notice = '';
		try {
			await fn();
			notice = msg;
			await load();
		} catch (e) {
			error = errorMessage(e);
		}
	}

	const save = () => run(() => patch(`/api/admin/groups/${id}`, { name, description }), 'Saved');
	const addMembers = () => run(() => post(`/api/admin/groups/${id}/members`, { user_ids: addUsers }).then(() => (addUsers = [])));
	const removeMember = (uid: string) => run(() => del(`/api/admin/groups/${id}/members/${uid}`));
	const grant = () => run(() => post(`/api/admin/groups/${id}/grants`, { model_ids: addModels }).then(() => (addModels = [])));
	const revoke = (gid: string) => run(() => del(`/api/admin/grants/${gid}`));
	const setApiKeys = (v: string) => run(() => patch(`/api/admin/groups/${id}`, { allow_api_keys: v === 'inherit' ? null : v === 'true' }), 'Saved');

	async function remove() {
		if (!confirm(`Delete group ${data.group.name}? Its model grants are removed; members keep access granted other ways.`)) return;
		await run(() => del(`/api/admin/groups/${id}`));
		goto('/admin/groups');
	}
</script>

<a href="/admin/groups" class="text-sm text-zinc-500 hover:underline">← Groups</a>
{#if error}<p class="error mt-4" role="alert">{error}</p>{/if}
{#if notice}<p class="mt-4 text-sm text-green-700 dark:text-green-400" role="status">{notice}</p>{/if}

{#if data}
	{@const g = data.group}
	<section class="card mt-4">
		<div class="grid gap-4 sm:grid-cols-2">
			<div><label class="label" for="gn">Name</label><input id="gn" class="input" bind:value={name} disabled={g.is_everyone} /></div>
			<div><label class="label" for="gd">Description</label><input id="gd" class="input" bind:value={description} /></div>
		</div>
		<div class="mt-4 grid gap-4 sm:grid-cols-2">
			<div>
				<label class="label" for="gk">Members may create platform API keys</label>
				<select id="gk" class="input" value={g.allow_api_keys === null ? 'inherit' : String(g.allow_api_keys)} onchange={(e) => setApiKeys(e.currentTarget.value)}>
					<option value="inherit">Use global setting</option><option value="true">Allow</option><option value="false">Deny</option>
				</select>
			</div>
		</div>
		<div class="mt-4 flex justify-between">
			{#if !g.is_everyone}<button class="btn-danger" onclick={remove}>Delete group</button>{:else}<span></span>{/if}
			<button class="btn" onclick={save}>Save</button>
		</div>
	</section>

	<div class="mt-6 grid gap-6 lg:grid-cols-2">
		<section class="card">
			<h3 class="font-semibold">Members ({data.members.length})</h3>
			{#if g.is_everyone}
				<p class="muted mt-1">Contains every active user automatically.</p>
			{:else}
				<div class="mt-3">
					<MultiPick items={users.filter((u) => !data.members.some((m: any) => m.id === u.id)).map((u) => ({ id: u.id, label: u.name, hint: u.email }))} bind:selected={addUsers} placeholder="Add people" />
					<button class="btn mt-2" disabled={!addUsers.length} onclick={addMembers}>Add</button>
				</div>
			{/if}
			<ul class="mt-3 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
				{#each data.members as m (m.id)}
					<li class="flex items-center gap-2 py-2">
						<a class="font-medium hover:underline" href="/admin/users/{m.id}">{m.name}</a>
						<span class="text-xs text-zinc-500">{m.email}</span>
						<span class="flex-1"></span>
						{#if !g.is_everyone}<button class="text-xs text-red-600 hover:underline" onclick={() => removeMember(m.id)}>Remove</button>{/if}
					</li>
				{/each}
			</ul>
		</section>
		<section class="card">
			<h3 class="font-semibold">Model access</h3>
			<div class="mt-3">
				<MultiPick items={models.filter((m) => !data.grants.some((x: any) => x.model_id === m.id)).map((m) => ({ id: m.id, label: m.display_name, hint: m.provider_name }))} bind:selected={addModels} placeholder="Grant models" />
				<button class="btn mt-2" disabled={!addModels.length} onclick={grant}>Grant {addModels.length || ''}</button>
			</div>
			<ul class="mt-3 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
				{#each data.grants as x (x.grant_id)}
					<li class="flex items-center gap-2 py-2">
						<span class="font-medium">{x.display_name}</span><span class="text-xs text-zinc-500">{x.provider_name}</span>
						<span class="flex-1"></span>
						<button class="text-xs text-red-600 hover:underline" onclick={() => revoke(x.grant_id)}>Revoke</button>
					</li>
				{:else}
					<li class="muted py-2">No model grants. Public models are available to everyone anyway.</li>
				{/each}
			</ul>
		</section>
	</div>
{/if}
