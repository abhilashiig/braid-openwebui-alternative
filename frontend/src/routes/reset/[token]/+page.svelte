<script lang="ts">
	import { page } from '$app/state';
	import { post, errorMessage } from '#lib/api.ts';
	import AuthCard from '#lib/components/AuthCard.svelte';
	import PasswordFields from '#lib/components/PasswordFields.svelte';

	let password = $state('');
	let confirm = $state('');
	let error = $state('');
	let done = $state(false);
	let busy = $state(false);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (password !== confirm) return;
		busy = true;
		error = '';
		try {
			await post(`/api/auth/reset/${page.params.token}`, { password });
			done = true;
		} catch (err) {
			error = errorMessage(err);
		} finally {
			busy = false;
		}
	}
</script>

<AuthCard title="Set a new password">
	{#if done}
		<p>Your password was changed. <a class="underline" href="/login">Sign in</a></p>
	{:else}
		<form class="space-y-4" onsubmit={submit}>
			{#if error}<p class="error" role="alert">{error}</p>{/if}
			<PasswordFields bind:password bind:confirm />
			<button class="btn w-full" disabled={busy}>Change password</button>
		</form>
	{/if}
</AuthCard>
