<script lang="ts">
	import { ApiError, post, put } from '#lib/api.ts';
	import { PRESETS } from '#lib/presets.ts';

	type Provider = {
		id?: string;
		name: string;
		api_format: 'openai' | 'anthropic';
		base_url: string;
		headers: Record<string, string>;
		organization: string | null;
		project: string | null;
		key_strategy: string;
		timeout_secs: number;
		max_retries: number;
		enabled: boolean;
	};

	let { provider = null, onsaved }: { provider?: Provider | null; onsaved: (id: string) => void } = $props();

	const editing = !!provider?.id;
	let preset = $state(editing ? 'custom' : 'openai');
	let form = $state<Provider>(
		provider
			? { ...provider, headers: { ...provider.headers } }
			: { name: 'OpenAI', api_format: 'openai', base_url: PRESETS[0].base_url, headers: {}, organization: '', project: '', key_strategy: 'failover', timeout_secs: 120, max_retries: 2, enabled: true }
	);
	let keyLabel = $state('Default');
	let key = $state('');
	let headersText = $state(Object.entries(provider?.headers ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n'));
	let allowPrivate = $state(false);
	let showAdvanced = $state(false);
	let error = $state('');
	let privateHint = $state(false);
	let busy = $state(false);

	function applyPreset() {
		const p = PRESETS.find((p) => p.id === preset)!;
		if (p.id !== 'custom') form.name = p.name.replace(' (local)', '');
		form.api_format = p.api_format;
		form.base_url = p.base_url;
		allowPrivate = !!p.local;
	}

	function parseHeaders(): Record<string, string> {
		const out: Record<string, string> = {};
		for (const line of headersText.split('\n')) {
			const i = line.indexOf(':');
			if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
		}
		return out;
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		error = '';
		privateHint = false;
		const body = { ...form, headers: parseHeaders(), allow_private: allowPrivate };
		try {
			if (editing) {
				await put(`/api/admin/providers/${provider!.id}`, body);
				onsaved(provider!.id!);
			} else {
				const res = await post('/api/admin/providers', { ...body, keys: key ? [{ label: keyLabel, key }] : [] });
				onsaved(res.id);
			}
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
			privateHint = err instanceof ApiError && err.code === 'private_address';
		} finally {
			busy = false;
		}
	}
</script>

<form id="provider-form" class="space-y-4" onsubmit={submit}>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if !editing}
		<div>
			<label class="label" for="preset">Preset</label>
			<select id="preset" class="input" bind:value={preset} onchange={applyPreset}>
				{#each PRESETS as p}<option value={p.id}>{p.name}</option>{/each}
			</select>
		</div>
	{/if}
	<div class="grid gap-4 sm:grid-cols-2">
		<div>
			<label class="label" for="pname">Display name</label>
			<input id="pname" class="input" required bind:value={form.name} />
		</div>
		<div>
			<label class="label" for="format">API format</label>
			<select id="format" class="input" bind:value={form.api_format}>
				<option value="openai">OpenAI-compatible</option>
				<option value="anthropic">Anthropic Messages</option>
			</select>
		</div>
	</div>
	<div>
		<label class="label" for="base">Base URL</label>
		<input id="base" class="input font-mono" type="url" required bind:value={form.base_url} placeholder="https://api.example.com/v1" />
		<p class="muted mt-1">Include the version path, e.g. <code>/v1</code>.</p>
	</div>
	<label class="flex items-start gap-2 text-sm">
		<input type="checkbox" class="mt-0.5" bind:checked={allowPrivate} />
		<span>Local server on a private network (e.g. Ollama). Allows this host past the private-address block.</span>
	</label>
	{#if privateHint}<p class="text-sm text-amber-700 dark:text-amber-400">Tick the box above if this is your own local server.</p>{/if}
	{#if !editing}
		<div class="grid gap-4 sm:grid-cols-3">
			<div>
				<label class="label" for="klabel">Key label</label>
				<input id="klabel" class="input" bind:value={keyLabel} />
			</div>
			<div class="sm:col-span-2">
				<label class="label" for="key">API key</label>
				<input id="key" class="input font-mono" type="password" autocomplete="off" bind:value={key} placeholder={allowPrivate ? 'Optional for local servers' : 'sk-…'} />
			</div>
		</div>
	{/if}
	<button type="button" class="text-sm text-zinc-600 underline dark:text-zinc-400" onclick={() => (showAdvanced = !showAdvanced)}>
		{showAdvanced ? 'Hide' : 'Show'} advanced options
	</button>
	{#if showAdvanced}
		<div class="space-y-4">
			{#if form.api_format === 'openai'}
				<div class="grid gap-4 sm:grid-cols-2">
					<div>
						<label class="label" for="org">Organization ID</label>
						<input id="org" class="input" bind:value={form.organization} />
					</div>
					<div>
						<label class="label" for="proj">Project ID</label>
						<input id="proj" class="input" bind:value={form.project} />
					</div>
				</div>
			{/if}
			<div>
				<label class="label" for="headers">Custom headers</label>
				<textarea id="headers" class="input font-mono" rows="3" bind:value={headersText} placeholder="HTTP-Referer: https://example.com"></textarea>
			</div>
			<div class="grid gap-4 sm:grid-cols-3">
				<div>
					<label class="label" for="strategy">With several keys</label>
					<select id="strategy" class="input" bind:value={form.key_strategy}>
						<option value="failover">Primary with failover</option>
						<option value="round_robin">Round-robin</option>
					</select>
				</div>
				<div>
					<label class="label" for="timeout">Timeout (s)</label>
					<input id="timeout" class="input" type="number" min="1" max="3600" bind:value={form.timeout_secs} />
				</div>
				<div>
					<label class="label" for="retries">Retries on 429/5xx</label>
					<input id="retries" class="input" type="number" min="0" max="10" bind:value={form.max_retries} />
				</div>
			</div>
			<label class="flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={form.enabled} /> Enabled</label>
		</div>
	{/if}
	<div class="flex justify-end">
		<button class="btn" disabled={busy}>{busy ? 'Saving…' : editing ? 'Save' : 'Add provider'}</button>
	</div>
</form>
