<script lang="ts">
	import { page } from '$app/state';
	import { post } from '#lib/api.ts';
	import { loadSession, session } from '#lib/session.svelte.ts';
	import { applyTheme, currentTheme, type Theme } from '#lib/theme.ts';
	import ChatList from '#lib/components/ChatList.svelte';

	let { children } = $props();
	let sidebarOpen = $state(false);
	let theme = $state<Theme>(currentTheme());

	const isAdmin = $derived(session.me?.user.role === 'admin');

	async function logout() {
		await post('/api/auth/logout');
		await loadSession();
	}

	function cycleTheme() {
		theme = theme === 'system' ? 'light' : theme === 'light' ? 'dark' : 'system';
		applyTheme(theme);
	}

	$effect(() => {
		page.url.pathname;
		sidebarOpen = false;
	});
</script>

<div class="flex h-dvh overflow-hidden">
	{#if sidebarOpen}
		<button class="fixed inset-0 z-30 bg-black/40 md:hidden" aria-label="Close menu" onclick={() => (sidebarOpen = false)}></button>
	{/if}
	<aside
		class="fixed inset-y-0 left-0 z-40 flex w-72 flex-col border-r border-zinc-200 bg-zinc-50 transition-transform md:static md:translate-x-0 dark:border-zinc-800 dark:bg-zinc-900 {sidebarOpen
			? 'translate-x-0'
			: '-translate-x-full'}"
	>
		<div class="flex items-center gap-2 px-4 py-3 font-semibold">
			{#if session.instance?.logo_url}<img src={session.instance.logo_url} alt="" class="h-6 w-6 rounded" />{/if}
			<span class="truncate">{session.instance?.name}</span>
		</div>
		<div class="px-3">
			<a href="/" class="btn-secondary w-full justify-start">＋ New chat</a>
		</div>
		<nav class="mt-3 min-h-0 flex-1 overflow-y-auto px-3" aria-label="Chats">
			<ChatList />
		</nav>
		<div class="space-y-1 border-t border-zinc-200 p-3 text-sm dark:border-zinc-800">
			{#if isAdmin}
				<a href="/admin/providers" class="block rounded-lg px-3 py-2 hover:bg-zinc-200 dark:hover:bg-zinc-800" class:font-semibold={page.url.pathname.startsWith('/admin')}>Admin</a>
			{/if}
			<a href="/settings" class="block rounded-lg px-3 py-2 hover:bg-zinc-200 dark:hover:bg-zinc-800" class:font-semibold={page.url.pathname.startsWith('/settings')}>Settings</a>
			<button class="block w-full rounded-lg px-3 py-2 text-left hover:bg-zinc-200 dark:hover:bg-zinc-800" onclick={cycleTheme}>
				Theme: {theme}
			</button>
			<div class="flex items-center justify-between rounded-lg px-3 py-2">
				<div class="min-w-0">
					<div class="truncate font-medium">{session.me?.user.name}</div>
					<div class="truncate text-xs text-zinc-500">{session.me?.user.email}</div>
				</div>
				<button class="text-xs text-zinc-500 hover:underline" onclick={logout}>Sign out</button>
			</div>
		</div>
	</aside>

	<div class="flex min-w-0 flex-1 flex-col">
		<button class="m-2 self-start rounded-lg p-2 hover:bg-zinc-100 md:hidden dark:hover:bg-zinc-800" aria-label="Open menu" onclick={() => (sidebarOpen = true)}>☰</button>
		<div class="min-h-0 flex-1">
			{@render children()}
		</div>
	</div>
</div>
