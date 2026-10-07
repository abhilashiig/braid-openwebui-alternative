<script lang="ts">
	import { get, post, errorMessage } from '#lib/api.ts';
	import Modal from '#lib/components/Modal.svelte';

	let groups = $state<any[]>([]);
	let error = $state('');
	let open = $state(false);
	let name = $state('');
	let description = $state('');

	const load = () => get('/api/admin/groups').then((g) => (groups = g)).catch((e) => (error = errorMessage(e)));
	load();

	async function create(e: SubmitEvent) {
		e.preventDefault();
		try {
			await post('/api/admin/groups', { name, description });
			open = false;
			name = description = '';
			load();
		} catch (err) {
			error = errorMessage(err);
		}
	}
</script>

<div class="flex items-center justify-between">
	<p class="muted">Groups are the main way to grant model access.</p>
	<button class="btn" onclick={() => (open = true)}>New group</button>
</div>
{#if error}<p class="error mt-4">{error}</p>{/if}
<div class="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
	{#each groups as g (g.id)}
		<a href="/admin/groups/{g.id}" class="card block hover:border-zinc-400 dark:hover:border-zinc-600">
			<div class="font-semibold">{g.name}</div>
			{#if g.description}<p class="muted mt-1">{g.description}</p>{/if}
			<p class="mt-3 text-sm text-zinc-600 dark:text-zinc-400">{g.member_count} member{g.member_count === 1 ? '' : 's'} · {g.model_count} model grant{g.model_count === 1 ? '' : 's'}</p>
		</a>
	{/each}
</div>

<Modal bind:open title="New group">
	<form class="space-y-4" onsubmit={create}>
		<div><label class="label" for="g-name">Name</label><input id="g-name" class="input" required bind:value={name} /></div>
		<div><label class="label" for="g-desc">Description</label><input id="g-desc" class="input" bind:value={description} /></div>
		<div class="flex justify-end"><button class="btn">Create</button></div>
	</form>
</Modal>
