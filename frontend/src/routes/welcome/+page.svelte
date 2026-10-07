<script lang="ts">
	import { goto } from '$app/navigation';
	import { post, errorMessage } from '#lib/api.ts';
	import { session } from '#lib/session.svelte.ts';
	import ProviderForm from '#lib/components/ProviderForm.svelte';
	import ModelImport from '#lib/components/ModelImport.svelte';
	import SettingsForm from '#lib/components/SettingsForm.svelte';

	let step = $state(2);
	let providerId = $state<string | null>(null);
	let test = $state<{ ok: boolean; message: string } | null>(null);

	async function providerSaved(id: string) {
		providerId = id;
		test = { ok: false, message: 'Testing connection…' };
		try {
			test = await post(`/api/admin/providers/${id}/test`);
		} catch (e) {
			test = { ok: false, message: errorMessage(e) };
		}
	}

	$effect(() => {
		if (session.me && session.me.user.role !== 'admin') goto('/', { replaceState: true });
	});
</script>

<main class="min-h-dvh bg-zinc-50 px-4 py-10 dark:bg-zinc-950">
	<div class="mx-auto max-w-2xl">
		<p class="text-sm text-zinc-500">Step {step} of 3</p>
		{#if step === 2}
			<h1 class="mt-1 text-2xl font-semibold">Connect your first AI provider</h1>
			<p class="muted mt-1">Add a provider and its API key, test it, and import models. You can skip this and do it later under Admin → Providers.</p>
			<div class="card mt-6">
				{#if !providerId}
					<ProviderForm onsaved={providerSaved} />
				{:else}
					<p class="text-sm {test?.ok ? 'text-green-700 dark:text-green-400' : 'text-red-600'}" role="status">{test?.message}</p>
					{#if test?.ok}
						<h2 class="mt-4 mb-3 font-semibold">Choose models to import</h2>
						<ModelImport {providerId} ondone={() => (step = 3)} />
					{:else if test}
						<p class="muted mt-2">Fix it later under Admin → Providers.</p>
					{/if}
				{/if}
			</div>
			<div class="mt-4 flex justify-end"><button class="btn-secondary" onclick={() => (step = 3)}>{providerId ? 'Continue' : 'Skip'}</button></div>
		{:else}
			<h1 class="mt-1 text-2xl font-semibold">Name your instance</h1>
			<p class="muted mt-1">Instance name, logo, default model and how people join. All optional.</p>
			<div class="card mt-6"><SettingsForm compact onsaved={() => goto('/')} /></div>
			<div class="mt-4 flex justify-end"><button class="btn-secondary" onclick={() => goto('/')}>Skip</button></div>
		{/if}
	</div>
</main>
