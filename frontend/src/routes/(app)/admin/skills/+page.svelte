<script lang="ts">
	import { get, post, put, errorMessage } from '#lib/api.ts';
	import MultiPick from '#lib/components/MultiPick.svelte';

	let skills = $state<any[]>([]);
	let users = $state<any[]>([]);
	let groups = $state<any[]>([]);
	let forms = $state<Record<string, any>>({});
	let msg = $state<Record<string, { ok: boolean; text: string }>>({});

	async function load() {
		skills = await get('/api/admin/skills');
		for (const s of skills) {
			const g = await get(`/api/admin/skills/${s.name}/grants`);
			forms[s.name] = { enabled: s.enabled, visibility: s.visibility, secret: '', user_ids: g.user_ids, group_ids: g.group_ids };
		}
	}
	load();
	get('/api/admin/users').then((u) => (users = u));
	get('/api/admin/groups').then((g) => (groups = g));

	async function save(name: string) {
		const f = forms[name];
		try {
			await put(`/api/admin/skills/${name}`, { enabled: f.enabled, visibility: f.visibility, secret: f.secret || null });
			await put(`/api/admin/skills/${name}/grants`, { user_ids: f.user_ids, group_ids: f.group_ids });
			msg[name] = { ok: true, text: 'Saved' };
			await load();
		} catch (e) {
			msg[name] = { ok: false, text: errorMessage(e) };
		}
	}

	async function test(name: string) {
		msg[name] = { ok: true, text: 'Testing…' };
		try {
			const r = await post(`/api/admin/skills/${name}/test`);
			msg[name] = { ok: r.ok, text: r.message };
		} catch (e) {
			msg[name] = { ok: false, text: errorMessage(e) };
		}
	}
</script>

<p class="muted">Skills are tools models can call during a chat. They are offered only to models with tool calling enabled.</p>
{#each skills as s (s.name)}
	{@const f = forms[s.name]}
	{#if f}
		<section class="card mt-4">
			<div class="flex flex-wrap items-center justify-between gap-3">
				<div>
					<h2 class="font-semibold">{s.title}</h2>
					<p class="muted">{s.description}</p>
				</div>
				<label class="flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={f.enabled} /> Enabled</label>
			</div>
			<div class="mt-4 grid gap-4 sm:grid-cols-2">
				<div>
					<label class="label" for="sk-{s.name}">{s.secret_label}</label>
					<input id="sk-{s.name}" class="input font-mono" type="password" autocomplete="off" bind:value={f.secret} placeholder={s.secret_last4 ? `Saved (…${s.secret_last4}). Enter a new one to replace it.` : 'Paste the key'} />
					<p class="muted mt-1">Get one at <a class="underline" href="https://brave.com/search/api/" target="_blank" rel="noopener noreferrer">brave.com/search/api</a>.</p>
				</div>
				<fieldset>
					<legend class="label">Who can use it</legend>
					<div class="flex gap-4 text-sm">
						<label class="flex items-center gap-2"><input type="radio" value="public" bind:group={f.visibility} /> Everyone</label>
						<label class="flex items-center gap-2"><input type="radio" value="private" bind:group={f.visibility} /> Granted only</label>
					</div>
				</fieldset>
			</div>
			{#if f.visibility === 'private'}
				<div class="mt-4 grid gap-4 sm:grid-cols-2">
					<div><span class="label">Groups</span><MultiPick items={groups.map((g) => ({ id: g.id, label: g.name }))} bind:selected={f.group_ids} placeholder="Add group" /></div>
					<div><span class="label">Users</span><MultiPick items={users.map((u) => ({ id: u.id, label: u.name, hint: u.email }))} bind:selected={f.user_ids} placeholder="Add user" /></div>
				</div>
			{/if}
			<div class="mt-4 flex flex-wrap items-center justify-end gap-3">
				{#if msg[s.name]}<span class="text-sm {msg[s.name].ok ? 'text-green-700 dark:text-green-400' : 'text-red-600'}" role="status">{msg[s.name].text}</span>{/if}
				<button class="btn-secondary" onclick={() => test(s.name)} disabled={!s.secret_last4}>Test</button>
				<button class="btn" onclick={() => save(s.name)}>Save</button>
			</div>
		</section>
	{/if}
{/each}
