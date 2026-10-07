<script lang="ts">
	import { del, get, post, put, errorMessage } from '#lib/api.ts';
	import Modal from '#lib/components/Modal.svelte';
	import MultiPick from '#lib/components/MultiPick.svelte';

	let models = $state<any[]>([]);
	let providers = $state<any[]>([]);
	let users = $state<any[]>([]);
	let groups = $state<any[]>([]);
	let error = $state('');
	let q = $state('');
	let editOpen = $state(false);
	let form = $state<any>(null);
	let originalVisibility = '';
	let grantUsers = $state<string[]>([]);
	let grantGroups = $state<string[]>([]);
	let formError = $state('');
	let warning = $state('');

	const filtered = $derived(
		models.filter((m) => `${m.display_name} ${m.name} ${m.provider_name}`.toLowerCase().includes(q.toLowerCase()))
	);

	async function load() {
		try {
			[models, providers] = await Promise.all([get('/api/admin/models'), get('/api/admin/providers')]);
		} catch (e) {
			error = errorMessage(e);
		}
	}
	load();

	async function loadPeople() {
		if (!users.length) [users, groups] = await Promise.all([get('/api/admin/users'), get('/api/admin/groups')]);
	}

	async function openNew() {
		await loadPeople();
		form = {
			provider_id: providers[0]?.id ?? '', upstream_id: '', name: null, display_name: '', description: '', visibility: 'private', enabled: true,
			context_window: null, max_output_tokens: null, default_temperature: null, system_prompt: '', supports_vision: null, supports_tools: null,
			supports_reasoning: null, supports_streaming: true, price_input_per_m: null, price_output_per_m: null, forced_skills: []
		};
		grantUsers = [];
		grantGroups = [];
		formError = warning = '';
		originalVisibility = 'private';
		editOpen = true;
	}

	async function openEdit(m: any) {
		await loadPeople();
		form = { ...m, system_prompt: m.system_prompt ?? '' };
		originalVisibility = m.visibility;
		const g = await get(`/api/admin/models/${m.id}/grants`);
		grantUsers = g.users.map((u: any) => u.id);
		grantGroups = g.groups.map((x: any) => x.id);
		formError = warning = '';
		editOpen = true;
	}

	async function visibilityChanged() {
		warning = '';
		if (form.id && originalVisibility === 'public' && form.visibility === 'private') {
			const r = await get(`/api/admin/models/${form.id}/impact?visibility=private`);
			warning = r.users_losing_access
				? `${r.users_losing_access} user${r.users_losing_access === 1 ? '' : 's'} will lose access unless granted below.`
				: 'No users lose access.';
		}
	}

	const num = (v: any) => (v === '' || v == null ? null : Number(v));

	async function save(e: SubmitEvent) {
		e.preventDefault();
		formError = '';
		const body = {
			...form,
			context_window: num(form.context_window), max_output_tokens: num(form.max_output_tokens),
			default_temperature: num(form.default_temperature), price_input_per_m: num(form.price_input_per_m),
			price_output_per_m: num(form.price_output_per_m)
		};
		try {
			const id = form.id ?? (await post('/api/admin/models', body)).id;
			if (form.id) await put(`/api/admin/models/${id}`, body);
			await put(`/api/admin/models/${id}/grants`, { user_ids: grantUsers, group_ids: grantGroups });
			editOpen = false;
			load();
		} catch (err) {
			formError = errorMessage(err);
		}
	}

	async function remove() {
		if (!confirm(`Delete ${form.display_name}? Chats keep their history.`)) return;
		await del(`/api/admin/models/${form.id}`);
		editOpen = false;
		load();
	}

	async function toggle(m: any, field: 'enabled' | 'visibility') {
		const body = { ...m, [field]: field === 'enabled' ? !m.enabled : m.visibility === 'public' ? 'private' : 'public' };
		if (field === 'visibility' && body.visibility === 'private') {
			const r = await get(`/api/admin/models/${m.id}/impact?visibility=private`);
			if (r.users_losing_access && !confirm(`${r.users_losing_access} users will lose access to ${m.display_name}. Continue?`)) return;
		}
		await put(`/api/admin/models/${m.id}`, body).catch((e) => (error = errorMessage(e)));
		load();
	}
</script>

<div class="flex flex-wrap items-center justify-between gap-3">
	<input class="input max-w-xs" placeholder="Search models" bind:value={q} aria-label="Search models" />
	<button class="btn" onclick={openNew} disabled={!providers.length}>Add model manually</button>
</div>
{#if error}<p class="error mt-4">{error}</p>{/if}

<div class="mt-4 overflow-x-auto rounded-xl border border-zinc-200 dark:border-zinc-800">
	<table class="w-full text-sm">
		<thead class="bg-zinc-50 text-left text-xs text-zinc-500 uppercase dark:bg-zinc-900">
			<tr>
				<th class="px-4 py-2">Model</th>
				<th class="px-4 py-2">Provider</th>
				<th class="px-4 py-2">Capabilities</th>
				<th class="px-4 py-2">Visibility</th>
				<th class="px-4 py-2">Status</th>
				<th class="px-4 py-2"></th>
			</tr>
		</thead>
		<tbody class="divide-y divide-zinc-100 dark:divide-zinc-800">
			{#each filtered as m (m.id)}
				<tr>
					<td class="px-4 py-2">
						<div class="font-medium">{m.display_name}</div>
						<code class="text-xs text-zinc-500">{m.name}</code>
					</td>
					<td class="px-4 py-2">{m.provider_name}</td>
					<td class="px-4 py-2 text-xs text-zinc-500">
						{[m.supports_vision && 'vision', m.supports_tools && 'tools', m.supports_reasoning && 'reasoning'].filter(Boolean).join(' · ') || '–'}
					</td>
					<td class="px-4 py-2">
						<button class="rounded px-2 py-0.5 text-xs {m.visibility === 'public' ? 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200' : 'bg-zinc-100 dark:bg-zinc-800'}" onclick={() => toggle(m, 'visibility')} title="Click to switch">
							{m.visibility}{m.visibility === 'private' ? ` · ${m.grant_count} grant${m.grant_count === 1 ? '' : 's'}` : ''}
						</button>
					</td>
					<td class="px-4 py-2">
						<button class="text-xs hover:underline" onclick={() => toggle(m, 'enabled')}>{m.enabled ? 'Enabled' : 'Disabled'}</button>
					</td>
					<td class="px-4 py-2 text-right"><button class="text-sm underline" onclick={() => openEdit(m)}>Edit</button></td>
				</tr>
			{:else}
				<tr><td colspan="6" class="muted px-4 py-6 text-center">No models. Use “Fetch models” on a provider, or add one manually.</td></tr>
			{/each}
		</tbody>
	</table>
</div>

<Modal bind:open={editOpen} title={form?.id ? `Edit ${form.display_name}` : 'Add model'} wide>
	{#if form}
		<form id="model-form" class="space-y-4" onsubmit={save}>
			{#if formError}<p class="error">{formError}</p>{/if}
			<div class="grid gap-4 sm:grid-cols-2">
				<div>
					<label class="label" for="m-provider">Provider</label>
					<select id="m-provider" class="input" bind:value={form.provider_id} disabled={!!form.id}>
						{#each providers as p}<option value={p.id}>{p.name}</option>{/each}
					</select>
				</div>
				<div>
					<label class="label" for="m-up">Model ID (as the provider expects it)</label>
					<input id="m-up" class="input font-mono" required bind:value={form.upstream_id} placeholder="gpt-4o" />
				</div>
				<div>
					<label class="label" for="m-dn">Display name</label>
					<input id="m-dn" class="input" bind:value={form.display_name} />
				</div>
				<div>
					<label class="label" for="m-name">API name</label>
					<input id="m-name" class="input font-mono" bind:value={form.name} placeholder="provider/model-id (default)" required={!!form.id} />
				</div>
			</div>
			<div>
				<label class="label" for="m-desc">Description</label>
				<input id="m-desc" class="input" bind:value={form.description} />
			</div>
			<fieldset>
				<legend class="label">Visibility</legend>
				<div class="flex gap-4 text-sm">
					<label class="flex items-center gap-2"><input type="radio" value="private" bind:group={form.visibility} onchange={visibilityChanged} /> Private</label>
					<label class="flex items-center gap-2"><input type="radio" value="public" bind:group={form.visibility} onchange={visibilityChanged} /> Public (all active users)</label>
				</div>
				{#if warning}<p class="mt-1 text-sm text-amber-700 dark:text-amber-400">{warning}</p>{/if}
			</fieldset>
			<div class="grid gap-4 sm:grid-cols-2">
				<div>
					<span class="label">Grant to groups</span>
					<MultiPick items={groups.map((g) => ({ id: g.id, label: g.name }))} bind:selected={grantGroups} placeholder="Add group" />
				</div>
				<div>
					<span class="label">Grant to users</span>
					<MultiPick items={users.map((u) => ({ id: u.id, label: u.name, hint: u.email }))} bind:selected={grantUsers} placeholder="Add user" />
				</div>
			</div>
			<fieldset>
				<legend class="label">Capabilities</legend>
				<div class="flex flex-wrap gap-4 text-sm">
					<label class="flex items-center gap-2"><input type="checkbox" bind:checked={form.supports_vision} /> Vision</label>
					<label class="flex items-center gap-2"><input type="checkbox" bind:checked={form.supports_tools} /> Tool calling</label>
					<label class="flex items-center gap-2"><input type="checkbox" bind:checked={form.supports_reasoning} /> Reasoning</label>
					<label class="flex items-center gap-2"><input type="checkbox" bind:checked={form.supports_streaming} /> Streaming</label>
					<label class="flex items-center gap-2"><input type="checkbox" bind:checked={form.enabled} /> Enabled</label>
				</div>
				{#if !form.id}<p class="muted mt-1">Left unticked, capabilities are pre-filled from known model families.</p>{/if}
			</fieldset>
			<div class="grid gap-4 sm:grid-cols-3">
				<div>
					<label class="label" for="m-ctx">Context window</label>
					<input id="m-ctx" class="input" type="number" min="1" bind:value={form.context_window} />
				</div>
				<div>
					<label class="label" for="m-max">Max output tokens</label>
					<input id="m-max" class="input" type="number" min="1" bind:value={form.max_output_tokens} />
				</div>
				<div>
					<label class="label" for="m-temp">Default temperature</label>
					<input id="m-temp" class="input" type="number" min="0" max="2" step="0.1" bind:value={form.default_temperature} />
				</div>
				<div>
					<label class="label" for="m-pin">$ per 1M input tokens</label>
					<input id="m-pin" class="input" type="number" min="0" step="any" bind:value={form.price_input_per_m} />
				</div>
				<div>
					<label class="label" for="m-pout">$ per 1M output tokens</label>
					<input id="m-pout" class="input" type="number" min="0" step="any" bind:value={form.price_output_per_m} />
				</div>
				<div>
					<label class="flex items-center gap-2 pt-7 text-sm">
						<input type="checkbox" checked={form.forced_skills.includes('web_search')} onchange={(e) => (form.forced_skills = e.currentTarget.checked ? ['web_search'] : [])} />
						Always enable web search
					</label>
				</div>
			</div>
			<div>
				<label class="label" for="m-sys">Default system prompt</label>
				<textarea id="m-sys" class="input" rows="3" bind:value={form.system_prompt}></textarea>
			</div>
			<div class="flex justify-between">
				{#if form.id}<button type="button" class="btn-danger" onclick={remove}>Delete</button>{:else}<span></span>{/if}
				<button class="btn">Save</button>
			</div>
		</form>
	{/if}
</Modal>
