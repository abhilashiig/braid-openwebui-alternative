<script lang="ts">
	import { del, get, patch, post, errorMessage } from '#lib/api.ts';
	import Modal from '#lib/components/Modal.svelte';
	import ProviderForm from '#lib/components/ProviderForm.svelte';
	import ModelImport from '#lib/components/ModelImport.svelte';

	let providers = $state<any[]>([]);
	let error = $state('');
	let formOpen = $state(false);
	let editing = $state<any>(null);
	let importFor = $state<any>(null);
	let importOpen = $state(false);
	let keyFor = $state<any>(null);
	let keyOpen = $state(false);
	let newKey = $state({ label: '', key: '' });
	let tests = $state<Record<string, { ok: boolean; message: string; busy?: boolean }>>({});

	async function load() {
		try {
			providers = await get('/api/admin/providers');
		} catch (e) {
			error = errorMessage(e);
		}
	}
	load();

	function openAdd() {
		editing = null;
		formOpen = true;
	}

	async function saved(id: string) {
		const wasNew = !editing;
		formOpen = false;
		await load();
		if (wasNew) {
			await test(id);
			if (tests[id]?.ok) openImport(providers.find((p) => p.id === id));
		}
	}

	async function test(id: string) {
		tests[id] = { ok: false, message: 'Testing…', busy: true };
		try {
			const r = await post(`/api/admin/providers/${id}/test`);
			tests[id] = { ok: r.ok, message: r.message };
		} catch (e) {
			tests[id] = { ok: false, message: errorMessage(e) };
		}
		await load();
	}

	function openImport(p: any) {
		importFor = p;
		importOpen = true;
	}

	async function removeProvider(p: any) {
		const models = p.models.map((m: any) => m.display_name);
		const msg = models.length
			? `Delete ${p.name}? These ${models.length} models will also be deleted:\n\n${models.join('\n')}`
			: `Delete ${p.name}?`;
		if (!confirm(msg)) return;
		await del(`/api/admin/providers/${p.id}`).catch((e) => (error = errorMessage(e)));
		load();
	}

	async function addKey(e: SubmitEvent) {
		e.preventDefault();
		try {
			await post(`/api/admin/providers/${keyFor.id}/keys`, newKey);
			keyOpen = false;
			newKey = { label: '', key: '' };
			load();
		} catch (err) {
			error = errorMessage(err);
		}
	}

	async function keyPatch(id: string, body: any) {
		await patch(`/api/admin/provider-keys/${id}`, body).catch((e) => (error = errorMessage(e)));
		load();
	}

	async function removeKey(k: any) {
		if (!confirm(`Delete key "${k.label}" (…${k.last4})?`)) return;
		await del(`/api/admin/provider-keys/${k.id}`);
		load();
	}
</script>

<div class="flex items-center justify-between">
	<p class="muted">Upstream AI services. Keys are encrypted and never shown again after saving.</p>
	<button class="btn" onclick={openAdd}>Add provider</button>
</div>
{#if error}<p class="error mt-4" role="alert">{error}</p>{/if}

<div class="mt-6 space-y-4">
	{#each providers as p (p.id)}
		<section class="card">
			<div class="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h2 class="flex items-center gap-2 font-semibold">
						{p.name}
						<span class="rounded bg-zinc-100 px-1.5 py-0.5 text-xs font-normal dark:bg-zinc-800">{p.api_format === 'openai' ? 'OpenAI-compatible' : 'Anthropic'}</span>
						{#if !p.enabled}<span class="rounded bg-amber-100 px-1.5 py-0.5 text-xs font-normal text-amber-800 dark:bg-amber-900 dark:text-amber-200">Disabled</span>{/if}
					</h2>
					<p class="muted font-mono text-xs">{p.base_url}</p>
				</div>
				<div class="flex flex-wrap gap-2">
					<button class="btn-secondary" onclick={() => test(p.id)} disabled={tests[p.id]?.busy}>Test connection</button>
					<button class="btn-secondary" onclick={() => openImport(p)}>Fetch models</button>
					<button class="btn-secondary" onclick={() => ((editing = p), (formOpen = true))}>Edit</button>
					<button class="btn-secondary text-red-600" onclick={() => removeProvider(p)}>Delete</button>
				</div>
			</div>
			{#if tests[p.id]}
				<p class="mt-3 text-sm {tests[p.id].ok ? 'text-green-700 dark:text-green-400' : 'text-red-600'}" role="status">{tests[p.id].message}</p>
			{/if}
			<h3 class="mt-4 text-sm font-medium">API keys</h3>
			<ul class="mt-2 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
				{#each p.keys as k (k.id)}
					<li class="flex flex-wrap items-center gap-3 py-2">
						<span class="h-2 w-2 rounded-full {!k.enabled ? 'bg-zinc-400' : k.healthy ? 'bg-green-500' : 'bg-red-500'}" title={k.healthy ? 'Healthy' : 'Unhealthy'}></span>
						<span class="font-medium">{k.label}</span>
						<code class="text-zinc-500">…{k.last4}</code>
						{#if !k.healthy}<span class="text-xs text-red-600" title={k.last_error}>Rejected by provider</span>{/if}
						<span class="flex-1"></span>
						{#if !k.healthy}<button class="text-xs hover:underline" onclick={() => keyPatch(k.id, { healthy: true })}>Mark healthy</button>{/if}
						<button class="text-xs hover:underline" onclick={() => keyPatch(k.id, { enabled: !k.enabled })}>{k.enabled ? 'Disable' : 'Enable'}</button>
						<button class="text-xs text-red-600 hover:underline" onclick={() => removeKey(k)}>Delete</button>
					</li>
				{:else}
					<li class="muted py-2">No keys yet.</li>
				{/each}
			</ul>
			<button class="mt-2 text-sm underline" onclick={() => ((keyFor = p), (keyOpen = true))}>Add key</button>
			<p class="muted mt-3">{p.models.length} model{p.models.length === 1 ? '' : 's'} registered · <a class="underline" href="/admin/models">Manage</a></p>
		</section>
	{:else}
		<div class="card text-center">
			<p>No providers yet.</p>
			<p class="muted mt-1">Add OpenAI, Anthropic, OpenRouter, a local Ollama, or any OpenAI-compatible server.</p>
		</div>
	{/each}
</div>

<Modal bind:open={formOpen} title={editing ? `Edit ${editing.name}` : 'Add provider'} wide>
	<ProviderForm provider={editing} onsaved={saved} />
</Modal>

<Modal bind:open={importOpen} title={`Models from ${importFor?.name ?? ''}`} wide>
	{#if importFor}<ModelImport providerId={importFor.id} ondone={() => ((importOpen = false), load())} />{/if}
</Modal>

<Modal bind:open={keyOpen} title={`Add key to ${keyFor?.name ?? ''}`}>
	<form class="space-y-4" onsubmit={addKey}>
		<div>
			<label class="label" for="nk-label">Label</label>
			<input id="nk-label" class="input" bind:value={newKey.label} placeholder="Backup key" />
		</div>
		<div>
			<label class="label" for="nk-key">API key</label>
			<input id="nk-key" class="input font-mono" type="password" autocomplete="off" required bind:value={newKey.key} />
		</div>
		<div class="flex justify-end"><button class="btn">Add key</button></div>
	</form>
</Modal>
