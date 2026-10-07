<script lang="ts">
	import { get, post, put, errorMessage } from '#lib/api.ts';
	import { loadSession } from '#lib/session.svelte.ts';

	let { compact = false, onsaved }: { compact?: boolean; onsaved?: () => void } = $props();

	let s = $state<any>(null);
	let models = $state<any[]>([]);
	let error = $state('');
	let notice = $state('');
	let domains = $state('');
	let hosts = $state('');
	let smtpPassword = $state('');
	let testTo = $state('');
	let smtpMsg = $state<{ ok: boolean; text: string } | null>(null);

	async function saveSmtpPassword() {
		await put('/api/admin/settings/smtp-password', { password: smtpPassword });
		smtpPassword = '';
		s.smtp_password_set = true;
		smtpMsg = { ok: true, text: 'SMTP password saved' };
	}

	async function testSmtp() {
		smtpMsg = { ok: true, text: 'Sending…' };
		try {
			smtpMsg = await post('/api/admin/settings/smtp-test', { to: testTo }).then((r) => ({ ok: r.ok, text: r.message }));
		} catch (e) {
			smtpMsg = { ok: false, text: errorMessage(e) };
		}
	}

	get('/api/admin/settings').then((r) => {
		s = r;
		domains = r.signup_domains.join(', ');
		hosts = r.allowed_private_hosts.join(', ');
	});
	get('/api/admin/models').then((m) => (models = m.filter((x: any) => x.enabled)));

	const list = (t: string) => t.split(/[\s,]+/).map((x) => x.trim()).filter(Boolean);

	async function onLogo(e: Event) {
		const file = (e.currentTarget as HTMLInputElement).files?.[0];
		if (!file) return;
		if (file.size > 200_000) return (error = 'Logo must be under 200 KB');
		s.logo_url = await new Promise<string>((res) => {
			const r = new FileReader();
			r.onload = () => res(r.result as string);
			r.readAsDataURL(file);
		});
	}

	async function save(e: SubmitEvent) {
		e.preventDefault();
		error = notice = '';
		try {
			await put('/api/admin/settings', { ...s, signup_domains: list(domains), allowed_private_hosts: list(hosts), default_model_id: s.default_model_id || null });
			notice = 'Settings saved';
			await loadSession();
			onsaved?.();
		} catch (err) {
			error = errorMessage(err);
		}
	}
</script>

{#if s}
	<form class="space-y-6" onsubmit={save}>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div class="grid gap-4 sm:grid-cols-2">
			<div><label class="label" for="s-name">Instance name</label><input id="s-name" class="input" required bind:value={s.name} /></div>
			<div>
				<span class="label">Logo</span>
				<div class="flex items-center gap-3">
					{#if s.logo_url}<img src={s.logo_url} alt="Logo" class="h-9 w-9 rounded" />{/if}
					<label class="btn-secondary cursor-pointer">Upload<input type="file" accept="image/*" class="sr-only" onchange={onLogo} /></label>
					{#if s.logo_url}<button type="button" class="text-sm underline" onclick={() => (s.logo_url = null)}>Remove</button>{/if}
				</div>
			</div>
			<div>
				<label class="label" for="s-model">Default model</label>
				<select id="s-model" class="input" bind:value={s.default_model_id}>
					<option value={null}>None</option>
					{#each models as m}<option value={m.id}>{m.display_name} ({m.provider_name})</option>{/each}
				</select>
			</div>
			<div>
				<label class="label" for="s-signup">Sign-up</label>
				<select id="s-signup" class="input" bind:value={s.signup_mode}>
					<option value="invite">Invite only</option>
					<option value="open">Open, limited to email domains below</option>
				</select>
			</div>
			{#if s.signup_mode === 'open'}
				<div class="sm:col-span-2">
					<label class="label" for="s-domains">Allowed email domains</label>
					<input id="s-domains" class="input" bind:value={domains} placeholder="example.com, example.org (empty = any domain)" />
				</div>
			{/if}
		</div>
		{#if !compact}
			<div class="grid gap-4 sm:grid-cols-2">
				<label class="flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={s.allow_api_keys} /> Allow all users to create platform API keys</label>
				<div><label class="label" for="s-maxkeys">Max API keys per user</label><input id="s-maxkeys" class="input" type="number" min="0" max="100" bind:value={s.max_api_keys_per_user} /></div>
				<div><label class="label" for="s-rpm">Max requests/minute per API key</label><input id="s-rpm" class="input" type="number" min="1" bind:value={s.max_api_key_rpm} placeholder="No limit" /></div>
				<div><label class="label" for="s-ttl">Invitation validity (days)</label><input id="s-ttl" class="input" type="number" min="1" max="90" bind:value={s.invite_ttl_days} /></div>
				<div><label class="label" for="s-tools">Max tool calls per reply</label><input id="s-tools" class="input" type="number" min="1" max="20" bind:value={s.max_tool_calls} /></div>
				<label class="flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={s.show_metrics} /> Show speed and token metrics under replies by default</label>
				<label class="flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={s.log_content} /> Log prompt content for API requests</label>
				<div class="sm:col-span-2">
					<label class="label" for="s-hosts">Allowed private hosts</label>
					<input id="s-hosts" class="input font-mono" bind:value={hosts} placeholder="localhost, 10.0.0.5" />
					<p class="muted mt-1">Hosts on private networks that providers and page fetches may reach (e.g. a local Ollama).</p>
				</div>
			</div>
		{/if}
		{#if !compact}
			<fieldset class="space-y-4 rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
				<legend class="px-1 text-sm font-semibold">Outbound email (SMTP)</legend>
				<p class="muted">Used for invitations and password resets. Without it, admins copy invite and reset links from the UI.</p>
				<div class="grid gap-4 sm:grid-cols-3">
					<div class="sm:col-span-2"><label class="label" for="s-host">SMTP host</label><input id="s-host" class="input" bind:value={s.smtp_host} placeholder="smtp.example.com" /></div>
					<div><label class="label" for="s-port">Port</label><input id="s-port" class="input" type="number" min="1" max="65535" bind:value={s.smtp_port} /></div>
					<div>
						<label class="label" for="s-tls">Security</label>
						<select id="s-tls" class="input" bind:value={s.smtp_tls}><option value="starttls">STARTTLS (587)</option><option value="tls">TLS (465)</option><option value="none">None (local relay only)</option></select>
					</div>
					<div><label class="label" for="s-user">Username</label><input id="s-user" class="input" autocomplete="off" bind:value={s.smtp_username} /></div>
					<div><label class="label" for="s-from">From address</label><input id="s-from" class="input" bind:value={s.smtp_from} placeholder="Braid <noreply@example.com>" /></div>
				</div>
				<div class="flex flex-wrap items-end gap-3">
					<div class="min-w-60 flex-1">
						<label class="label" for="s-pass">Password</label>
						<input id="s-pass" class="input" type="password" autocomplete="new-password" bind:value={smtpPassword} placeholder={s.smtp_password_set ? 'Saved. Enter a new one to replace it.' : ''} />
					</div>
					<button type="button" class="btn-secondary" disabled={!smtpPassword} onclick={saveSmtpPassword}>Save password</button>
				</div>
				<div class="flex flex-wrap items-end gap-3">
					<div class="min-w-60 flex-1"><label class="label" for="s-test">Send a test email to</label><input id="s-test" class="input" type="email" bind:value={testTo} /></div>
					<button type="button" class="btn-secondary" disabled={!testTo} onclick={testSmtp}>Send test</button>
				</div>
				<p class="muted">Save settings before testing.</p>
				{#if smtpMsg}<p class="text-sm {smtpMsg.ok ? 'text-green-700 dark:text-green-400' : 'text-red-600'}" role="status">{smtpMsg.text}</p>{/if}
			</fieldset>
			{#if s.env_overrides?.length}
				<p class="text-sm text-amber-700 dark:text-amber-400">Set by environment variables (they win over values saved here): {s.env_overrides.join(', ')}</p>
			{/if}
		{/if}
		<div class="flex items-center justify-end gap-3">
			{#if notice}<span class="text-sm text-green-700 dark:text-green-400" role="status">{notice}</span>{/if}
			<button class="btn">Save settings</button>
		</div>
	</form>
{/if}
