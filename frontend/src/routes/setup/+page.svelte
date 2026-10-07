<script lang="ts">
	import { post, errorMessage } from '#lib/api.ts';
	import { loadSession } from '#lib/session.svelte.ts';
	import AuthCard from '#lib/components/AuthCard.svelte';
	import PasswordFields from '#lib/components/PasswordFields.svelte';

	let token = $state('');
	let name = $state('');
	let email = $state('');
	let password = $state('');
	let confirm = $state('');
	let error = $state('');
	let busy = $state(false);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (password !== confirm) return;
		busy = true;
		error = '';
		try {
			await post('/api/setup', { token, name, email, password });
			await loadSession();
		} catch (err) {
			error = errorMessage(err);
		} finally {
			busy = false;
		}
	}
</script>

<AuthCard title="Welcome to Braid" subtitle="Create the first admin account for this instance.">
	<form class="space-y-4" onsubmit={submit}>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div>
			<label class="label" for="token">Setup token</label>
			<input id="token" class="input font-mono" required bind:value={token} autocomplete="off" aria-describedby="token-hint" />
			<p id="token-hint" class="muted mt-1">Printed in the server logs at startup, or set with <code>BRAID_SETUP_TOKEN</code>.</p>
		</div>
		<div>
			<label class="label" for="name">Your name</label>
			<input id="name" class="input" autocomplete="name" required bind:value={name} />
		</div>
		<div>
			<label class="label" for="email">Email</label>
			<input id="email" class="input" type="email" autocomplete="email" required bind:value={email} />
		</div>
		<PasswordFields bind:password bind:confirm />
		<button class="btn w-full" disabled={busy}>{busy ? 'Creating…' : 'Create admin account'}</button>
	</form>
</AuthCard>
