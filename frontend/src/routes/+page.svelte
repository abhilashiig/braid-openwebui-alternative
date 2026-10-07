<script lang="ts">
	import { post } from '#lib/api';
	import { loadSession, session } from '#lib/session.svelte';

	async function logout() {
		await post('/api/auth/logout');
		await loadSession();
	}
</script>

<main class="grid min-h-dvh place-items-center p-4">
	<div class="card max-w-md text-center">
		<p>Signed in as <strong>{session.me?.user.name}</strong> ({session.me?.user.role})</p>
		<button class="btn-secondary mt-4" onclick={logout}>Sign out</button>
	</div>
</main>
