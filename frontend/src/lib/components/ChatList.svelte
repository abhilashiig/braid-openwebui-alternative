<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { del, patch } from '#lib/api.ts';
	import { refreshChats, store } from '#lib/chats.svelte.ts';

	let editing = $state<string | null>(null);
	let draft = $state('');

	$effect(() => {
		store.q;
		const t = setTimeout(() => refreshChats().catch(() => {}), store.q ? 250 : 0);
		return () => clearTimeout(t);
	});

	function groupOf(iso: string) {
		const d = (Date.now() - new Date(iso).getTime()) / 86400000;
		return d < 1 ? 'Today' : d < 2 ? 'Yesterday' : d < 7 ? 'Previous 7 days' : d < 30 ? 'Previous 30 days' : 'Older';
	}

	async function rename(id: string) {
		if (draft.trim()) await patch(`/api/chats/${id}`, { title: draft });
		editing = null;
		refreshChats();
	}

	async function remove(id: string, title: string) {
		if (!confirm(`Delete “${title}”?`)) return;
		await del(`/api/chats/${id}`);
		if (page.url.pathname === `/c/${id}`) goto('/');
		refreshChats();
	}
</script>

<input class="input mb-2" placeholder="Search chats" bind:value={store.q} aria-label="Search chats" />
{#each store.chats as c, i (c.id)}
	{#if i === 0 || groupOf(store.chats[i - 1].updated_at) !== groupOf(c.updated_at)}
		<div class="px-2 pt-3 pb-1 text-xs font-medium text-zinc-500">{groupOf(c.updated_at)}</div>
	{/if}
	{#if editing === c.id}
		<form onsubmit={(e) => (e.preventDefault(), rename(c.id))}>
			<!-- svelte-ignore a11y_autofocus -->
			<input class="input py-1" bind:value={draft} autofocus onblur={() => rename(c.id)} onkeydown={(e) => e.key === 'Escape' && (editing = null)} aria-label="Chat title" />
		</form>
	{:else}
		<div class="group flex items-center rounded-lg text-sm hover:bg-zinc-200 dark:hover:bg-zinc-800 {page.url.pathname === `/c/${c.id}` ? 'bg-zinc-200 dark:bg-zinc-800' : ''}">
			<a href="/c/{c.id}" class="min-w-0 flex-1 truncate px-2 py-1.5" title={c.title}>{c.title}</a>
			<div class="hidden shrink-0 gap-0.5 pr-1 group-focus-within:flex group-hover:flex">
				<button class="rounded px-1 text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100" aria-label="Rename {c.title}" onclick={() => ((editing = c.id), (draft = c.title))}>✎</button>
				<button class="rounded px-1 text-xs text-zinc-500 hover:text-red-600" aria-label="Delete {c.title}" onclick={() => remove(c.id, c.title)}>🗑</button>
			</div>
		</div>
	{/if}
{:else}
	<p class="muted px-2">{store.q ? 'No matching chats.' : 'No chats yet.'}</p>
{/each}
