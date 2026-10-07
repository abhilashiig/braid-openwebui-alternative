<script lang="ts">
	import { post, errorMessage } from '#lib/api.ts';
	import AuthCard from '#lib/components/AuthCard.svelte';

	let email = $state('');
	let sent = $state(false);
	let error = $state('');

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		try {
			await post('/api/auth/forgot', { email });
			sent = true;
		} catch (err) {
			error = errorMessage(err);
		}
	}
</script>

<AuthCard title="Reset your password">
	{#if sent}
		<p>If an account exists for <strong>{email}</strong>, we've emailed a reset link. It expires in 24 hours.</p>
		<p class="mt-4"><a class="underline" href="/login">Back to sign in</a></p>
	{:else}
		<form class="space-y-4" onsubmit={submit}>
			{#if error}<p class="error" role="alert">{error}</p>{/if}
			<div><label class="label" for="email">Email</label><input id="email" class="input" type="email" required bind:value={email} /></div>
			<button class="btn w-full">Email me a reset link</button>
			<p class="muted text-center"><a class="underline" href="/login">Back to sign in</a></p>
		</form>
	{/if}
</AuthCard>
