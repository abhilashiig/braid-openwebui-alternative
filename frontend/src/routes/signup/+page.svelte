<script lang="ts">
	import { post, errorMessage } from '#lib/api';
	import { loadSession } from '#lib/session.svelte';
	import AuthCard from '#lib/components/AuthCard.svelte';
	import PasswordFields from '#lib/components/PasswordFields.svelte';

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
			await post('/api/auth/signup', { name, email, password });
			await loadSession();
		} catch (err) {
			error = errorMessage(err);
		} finally {
			busy = false;
		}
	}
</script>

<AuthCard title="Create your account">
	<form class="space-y-4" onsubmit={submit}>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div>
			<label class="label" for="name">Name</label>
			<input id="name" class="input" autocomplete="name" required bind:value={name} />
		</div>
		<div>
			<label class="label" for="email">Email</label>
			<input id="email" class="input" type="email" autocomplete="email" required bind:value={email} />
		</div>
		<PasswordFields bind:password bind:confirm />
		<button class="btn w-full" disabled={busy}>{busy ? 'Creating…' : 'Create account'}</button>
		<p class="muted text-center">Have an account? <a class="underline" href="/login">Sign in</a></p>
	</form>
</AuthCard>
