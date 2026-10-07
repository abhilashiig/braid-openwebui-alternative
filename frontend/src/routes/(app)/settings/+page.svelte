<script lang="ts">
	import { del, get, patch, post, errorMessage } from '#lib/api.ts';
	import { loadSession, session } from '#lib/session.svelte.ts';
	import { loadModels, store } from '#lib/chats.svelte.ts';
	import { fmtCost, fmtNum, timeAgo } from '#lib/format.ts';
	import CopyButton from '#lib/components/CopyButton.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import MultiPick from '#lib/components/MultiPick.svelte';

	let name = $state(session.me?.user.name ?? '');
	let pw = $state({ current: '', new: '', confirm: '' });
	let defaultModel = $state(session.me?.default_model_id ?? '');
	let showMetrics = $state(true);
	let msg = $state<Record<string, { ok: boolean; text: string }>>({});
	let keys = $state<any>(null);
	let usage = $state<any[]>([]);
	let newOpen = $state(false);
	let created = $state<string | null>(null);
	let nk = $state({ name: '', expires_in_days: '', model_ids: [] as string[], rate_limit_rpm: '', token_quota_month: '' });

	try {
		const v = localStorage.getItem('showMetrics');
		showMetrics = v === null ? (session.me?.show_metrics ?? true) : v === '1';
	} catch {}

	const loadKeys = () => get('/api/me/api-keys').then((k) => (keys = k));
	$effect(() => {
		if (session.me?.user.must_enroll_2fa) return;
		if (!store.modelsLoaded) loadModels();
		loadKeys();
		get('/api/me/usage').then((u) => (usage = u.data));
	});

	async function act(section: string, fn: () => Promise<any>, ok: string) {
		try {
			await fn();
			msg[section] = { ok: true, text: ok };
		} catch (e) {
			msg[section] = { ok: false, text: errorMessage(e) };
		}
	}

	const saveProfile = (e: SubmitEvent) => (e.preventDefault(), act('profile', async () => { await patch('/api/me', { name }); await loadSession(); }, 'Saved'));
	const savePrefs = () => act('prefs', async () => {
		await patch('/api/me/preferences', { default_model_id: defaultModel || null });
		try { localStorage.setItem('showMetrics', showMetrics ? '1' : '0'); } catch {}
		await loadSession();
	}, 'Saved');
	function changePassword(e: SubmitEvent) {
		e.preventDefault();
		if (pw.new !== pw.confirm) return (msg.pw = { ok: false, text: "New passwords don't match" });
		act('pw', async () => { await post('/api/auth/password', { current: pw.current, new: pw.new }); pw = { current: '', new: '', confirm: '' }; }, 'Password changed');
	}

	async function createKey(e: SubmitEvent) {
		e.preventDefault();
		const num = (v: string) => (v === '' || v == null ? null : Number(v));
		try {
			const r = await post('/api/me/api-keys', {
				name: nk.name, expires_in_days: num(nk.expires_in_days), model_ids: nk.model_ids.length ? nk.model_ids : null,
				rate_limit_rpm: num(nk.rate_limit_rpm), token_quota_month: num(nk.token_quota_month)
			});
			created = r.key;
			nk = { name: '', expires_in_days: '', model_ids: [], rate_limit_rpm: '', token_quota_month: '' };
			loadKeys();
		} catch (err) {
			msg.key = { ok: false, text: errorMessage(err) };
		}
	}

	async function revoke(k: any) {
		if (!confirm(`Revoke key “${k.name}”? Apps using it stop working immediately.`)) return;
		await del(`/api/me/api-keys/${k.id}`);
		loadKeys();
	}

	let totp = $state<{ secret: string; otpauth_url: string; qr: string } | null>(null);
	let totpCode = $state('');
	let totpPassword = $state('');

	async function startTotp() {
		const r = await post('/api/me/2fa/setup');
		const QR = await import('qrcode');
		totp = { ...r, qr: await QR.toDataURL(r.otpauth_url, { margin: 1, width: 200 }) };
	}

	const enableTotp = (e: SubmitEvent) => (e.preventDefault(), act('totp', async () => {
		await post('/api/me/2fa/enable', { code: totpCode });
		totp = null;
		totpCode = '';
		await loadSession();
	}, 'Two-factor authentication is on'));

	const disableTotp = (e: SubmitEvent) => (e.preventDefault(), act('totp', async () => {
		await post('/api/me/2fa/disable', { password: totpPassword });
		totpPassword = '';
		await loadSession();
	}, 'Two-factor authentication is off'));

	const active = (k: any) => !k.revoked_at && (!k.expires_at || new Date(k.expires_at) > new Date());
</script>

{#snippet note(section: string)}
	{#if msg[section]}<span class="text-sm {msg[section].ok ? 'text-green-700 dark:text-green-400' : 'text-red-600'}" role="status">{msg[section].text}</span>{/if}
{/snippet}

<div class="h-full overflow-y-auto">
	<div class="mx-auto max-w-3xl space-y-6 px-4 py-6">
		<h1 class="text-2xl font-semibold">Settings</h1>

		{#if !session.me?.user.must_enroll_2fa}
		<section class="card">
			<h2 class="font-semibold">Profile</h2>
			<form class="mt-4 flex flex-wrap items-end gap-3" onsubmit={saveProfile}>
				<div class="min-w-60 flex-1"><label class="label" for="p-name">Name</label><input id="p-name" class="input" required bind:value={name} /></div>
				<div class="min-w-60 flex-1"><span class="label">Email</span><p class="py-2 text-sm">{session.me?.user.email}</p></div>
				<button class="btn">Save</button>
				{@render note('profile')}
			</form>
		</section>

		<section class="card">
			<h2 class="font-semibold">Preferences</h2>
			<div class="mt-4 grid gap-4 sm:grid-cols-2">
				<div>
					<label class="label" for="p-model">Default model for new chats</label>
					<select id="p-model" class="input" bind:value={defaultModel}>
						<option value="">Organization default</option>
						{#each store.models as m}<option value={m.id}>{m.display_name} ({m.provider_name})</option>{/each}
					</select>
				</div>
				<label class="flex items-center gap-2 pt-6 text-sm"><input type="checkbox" bind:checked={showMetrics} /> Show speed and token metrics under replies</label>
			</div>
			<div class="mt-4 flex items-center justify-end gap-3">{@render note('prefs')}<button class="btn" onclick={savePrefs}>Save</button></div>
		</section>

		<section class="card">
			<h2 class="font-semibold">Password</h2>
			<form class="mt-4 grid gap-4 sm:grid-cols-3" onsubmit={changePassword}>
				<div><label class="label" for="pw-c">Current</label><input id="pw-c" class="input" type="password" autocomplete="current-password" required bind:value={pw.current} /></div>
				<div><label class="label" for="pw-n">New (12+ characters)</label><input id="pw-n" class="input" type="password" autocomplete="new-password" minlength="12" required bind:value={pw.new} /></div>
				<div><label class="label" for="pw-r">Repeat new</label><input id="pw-r" class="input" type="password" autocomplete="new-password" required bind:value={pw.confirm} /></div>
				<div class="flex items-center justify-end gap-3 sm:col-span-3">{@render note('pw')}<button class="btn">Change password</button></div>
			</form>
		</section>

		{/if}
		<section class="card" id="two-factor">
			<h2 class="font-semibold">Two-factor authentication</h2>
			{#if session.me?.user.must_enroll_2fa}
				<p class="mt-2 rounded-lg bg-amber-50 p-3 text-sm text-amber-800 dark:bg-amber-950 dark:text-amber-200" role="alert">Your organization requires two-factor authentication. Set it up to continue.</p>
			{/if}
			{#if session.me?.user.totp_enabled}
				<p class="mt-2 text-sm text-green-700 dark:text-green-400">On. Sign-in asks for a code from your authenticator app.</p>
				<form class="mt-4 flex flex-wrap items-end gap-3" onsubmit={disableTotp}>
					<div class="min-w-60 flex-1"><label class="label" for="t-pw">Password to turn it off</label><input id="t-pw" class="input" type="password" autocomplete="current-password" required bind:value={totpPassword} /></div>
					<button class="btn-secondary">Turn off</button>
				</form>
			{:else if totp}
				<div class="mt-4 flex flex-wrap gap-6">
					<img src={totp.qr} alt="QR code for your authenticator app" class="h-40 w-40 rounded bg-white p-1" />
					<form class="min-w-60 flex-1 space-y-3" onsubmit={enableTotp}>
						<p class="text-sm">Scan the code with an authenticator app (1Password, Google Authenticator, Authy…), or enter this key:</p>
						<code class="block rounded bg-zinc-100 p-2 text-sm break-all dark:bg-zinc-800">{totp.secret}</code>
						<div><label class="label" for="t-code">6-digit code from the app</label><input id="t-code" class="input font-mono tracking-widest" inputmode="numeric" autocomplete="one-time-code" maxlength="6" required bind:value={totpCode} /></div>
						<button class="btn">Turn on</button>
					</form>
				</div>
			{:else}
				<p class="muted mt-2">Off. Add a code from an authenticator app to every sign-in.</p>
				<button class="btn mt-4" onclick={startTotp}>Set up</button>
			{/if}
			<div class="mt-2">{@render note('totp')}</div>
		</section>

		{#if !session.me?.user.must_enroll_2fa}
		<section class="card">
			<div class="flex flex-wrap items-center justify-between gap-2">
				<div>
					<h2 class="font-semibold">API keys</h2>
					<p class="muted">Call every model you can use through one OpenAI- or Anthropic-compatible API. <a class="underline" href="/settings/api">How to use the API</a></p>
				</div>
				{#if keys?.allowed}<button class="btn" onclick={() => ((created = null), (newOpen = true))}>Create key</button>{/if}
			</div>
			{#if keys && !keys.allowed}
				<p class="muted mt-4">API keys are not enabled for your account. Ask an admin.</p>
			{:else if keys}
				{@render note('key')}
				<ul class="mt-4 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
					{#each keys.keys as k (k.id)}
						<li class="flex flex-wrap items-center gap-3 py-2 {active(k) ? '' : 'opacity-50'}">
							<span class="font-medium">{k.name}</span>
							<code class="text-xs">{k.prefix}…</code>
							<span class="text-xs text-zinc-500">
								{#if k.revoked_at}revoked{:else if !active(k)}expired{:else}last used {timeAgo(k.last_used_at)}{k.last_used_ip ? ` from ${k.last_used_ip}` : ''}{/if}
								{#if k.expires_at && active(k)} · expires {new Date(k.expires_at).toLocaleDateString()}{/if}
								{#if k.model_ids} · {k.model_ids.length} model(s){/if}
								{#if k.rate_limit_rpm} · {k.rate_limit_rpm}/min{/if}
							</span>
							<span class="flex-1"></span>
							{#if active(k)}<button class="text-xs text-red-600 hover:underline" onclick={() => revoke(k)}>Revoke</button>{/if}
						</li>
					{:else}
						<li class="muted py-2">No keys yet.</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section class="card">
			<h2 class="font-semibold">Your usage (last 30 days)</h2>
			<div class="mt-3 overflow-x-auto">
				<table class="w-full text-sm">
					<thead class="text-left text-xs text-zinc-500 uppercase"><tr><th class="py-1">Day</th><th>Model</th><th class="text-right">Requests</th><th class="text-right">In</th><th class="text-right">Out</th><th class="text-right">Cost</th></tr></thead>
					<tbody>
						{#each usage as u}
							<tr class="border-t border-zinc-100 dark:border-zinc-800"><td class="py-1">{u.date}</td><td>{u.model}</td><td class="text-right">{u.requests}</td><td class="text-right">{fmtNum(u.input_tokens)}</td><td class="text-right">{fmtNum(u.output_tokens)}</td><td class="text-right">{fmtCost(u.cost)}</td></tr>
						{:else}
							<tr><td colspan="6" class="muted py-2">No usage yet.</td></tr>
						{/each}
					</tbody>
				</table>
			</div>
		</section>
		{/if}
	</div>
</div>

<Modal bind:open={newOpen} title={created ? 'Your new API key' : 'Create API key'}>
	{#if created}
		<p class="text-sm">Copy it now. You won't be able to see it again.</p>
		<div class="mt-3 flex items-center gap-2 rounded-lg bg-zinc-100 p-2 dark:bg-zinc-800"><code class="min-w-0 flex-1 text-sm break-all">{created}</code><CopyButton text={created} /></div>
		<div class="mt-4 flex justify-end"><button class="btn" onclick={() => (newOpen = false)}>Done</button></div>
	{:else}
		<form class="space-y-4" onsubmit={createKey}>
			<div><label class="label" for="k-name">Name</label><input id="k-name" class="input" required bind:value={nk.name} placeholder="My script" /></div>
			<div class="grid grid-cols-2 gap-4">
				<div><label class="label" for="k-exp">Expires in (days)</label><input id="k-exp" class="input" type="number" min="1" bind:value={nk.expires_in_days} placeholder="Never" /></div>
				<div><label class="label" for="k-rpm">Requests per minute</label><input id="k-rpm" class="input" type="number" min="1" max={keys?.max_rpm ?? undefined} bind:value={nk.rate_limit_rpm} placeholder={keys?.max_rpm ? `Max ${keys.max_rpm}` : 'No limit'} /></div>
				<div class="col-span-2"><label class="label" for="k-quota">Monthly token quota</label><input id="k-quota" class="input" type="number" min="1" bind:value={nk.token_quota_month} placeholder="No limit" /></div>
			</div>
			<div>
				<span class="label">Limit to models (optional)</span>
				<MultiPick items={store.models.map((m) => ({ id: m.id, label: m.display_name, hint: m.provider_name }))} bind:selected={nk.model_ids} placeholder="All my models" />
			</div>
			<div class="flex justify-end"><button class="btn">Create key</button></div>
		</form>
	{/if}
</Modal>
