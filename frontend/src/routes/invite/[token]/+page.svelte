<script lang="ts">
	import { page } from '$app/state';
	import { get, post, errorMessage } from '#lib/api';
	import { loadSession } from '#lib/session.svelte';
	import AuthCard from '#lib/components/AuthCard.svelte';
	import PasswordFields from '#lib/components/PasswordFields.svelte';

	const token = page.params.token!;
	let invite = $state<{ email: string } | null>(null);
	let loadError = $state('');
	let name = $state('');
	let password = $state('');
	let confirm = $state('');
	let error = $state('');
	let busy = $state(false);

	get(`/api/auth/invite/${token}`).then((i) => (invite = i)).catch((e) => (loadError = errorMessage(e)));

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (password !== confirm) return;
		busy = true;
		error = '';
		try {
			await post(`/api/auth/invite/${token}`, { name, password });
			await loadSession();
		} catch (err) {
			error = errorMessage(err);
		} finally {
			busy = false;
		}
	}
</script>

<AuthCard title="Accept invitation" subtitle={invite ? `You're joining as ${invite.email}` : undefined}>
	{#if loadError}
		<p class="error">{loadError}</p>
	{:else if invite}
		<form class="space-y-4" onsubmit={submit}>
			{#if error}<p class="error" role="alert">{error}</p>{/if}
			<div>
				<label class="label" for="name">Your name</label>
				<input id="name" class="input" autocomplete="name" required bind:value={name} />
			</div>
			<PasswordFields bind:password bind:confirm />
			<button class="btn w-full" disabled={busy}>{busy ? 'Activating…' : 'Activate account'}</button>
		</form>
	{/if}
</AuthCard>
