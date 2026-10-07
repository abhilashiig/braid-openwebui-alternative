<script lang="ts">
	import { ApiError, post, errorMessage } from '#lib/api.ts';
	import { loadSession, session } from '#lib/session.svelte.ts';
	import AuthCard from '#lib/components/AuthCard.svelte';

	let email = $state('');
	let password = $state('');
	let code = $state('');
	let needCode = $state(false);
	let error = $state('');
	let busy = $state(false);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		error = '';
		try {
			await post('/api/auth/login', { email, password, code: needCode ? code : undefined });
			await loadSession();
		} catch (err) {
			if (err instanceof ApiError && err.code === 'totp_required') {
				needCode = true;
				return;
			}
			error = errorMessage(err);
		} finally {
			busy = false;
		}
	}
</script>

<AuthCard title="Sign in">
	<form class="space-y-4" onsubmit={submit}>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div>
			<label class="label" for="email">Email</label>
			<input id="email" class="input" type="email" autocomplete="username" required bind:value={email} />
		</div>
		<div>
			<label class="label" for="password">Password</label>
			<input id="password" class="input" type="password" autocomplete="current-password" required bind:value={password} />
		</div>
		{#if needCode}
			<div>
				<label class="label" for="code">Authenticator code</label>
				<!-- svelte-ignore a11y_autofocus -->
				<input id="code" class="input font-mono tracking-widest" inputmode="numeric" autocomplete="one-time-code" maxlength="6" required autofocus bind:value={code} />
			</div>
		{/if}
		<button class="btn w-full" disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
		{#if session.instance?.email_enabled}
			<p class="muted text-center"><a class="underline" href="/forgot">Forgot your password?</a></p>
		{:else}
			<p class="muted text-center">Forgot your password? Ask an admin for a reset link.</p>
		{/if}
		{#if session.instance?.open_signup}
			<p class="muted text-center">No account? <a class="underline" href="/signup">Sign up</a></p>
		{/if}
	</form>
</AuthCard>
