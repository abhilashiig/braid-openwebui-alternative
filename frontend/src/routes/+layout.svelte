<script lang="ts">
	import '../app.css';
	import favicon from '#lib/assets/favicon.svg';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { loadSession, session } from '#lib/session.svelte.ts';
	import { applyTheme } from '#lib/theme.ts';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
	let loadError = $state('');

	const PUBLIC = ['/setup', '/login', '/signup', '/invite/', '/reset/'];

	applyTheme();
	loadSession().catch((e) => (loadError = e.message));

	const target = $derived.by(() => {
		if (!session.loaded) return null;
		const path = page.url.pathname;
		const isPublic = PUBLIC.some((p) => path === p || (p.endsWith('/') && path.startsWith(p)));
		if (session.instance?.needs_setup) return path === '/setup' ? null : '/setup';
		if (path === '/setup') return session.me ? '/welcome' : '/login';
		if (!session.me && !isPublic) return '/login';
		if (session.me && (path === '/login' || path === '/signup')) return '/';
		if (session.me && path.startsWith('/admin') && session.me.user.role !== 'admin') return '/';
		return null;
	});

	$effect(() => {
		if (target) goto(target, { replaceState: true });
	});
</script>

<svelte:head>
	<link rel="icon" href={session.instance?.logo_url || favicon} />
	<title>{session.instance?.name ?? 'Braid'}</title>
</svelte:head>

{#if loadError}
	<div class="grid min-h-dvh place-items-center p-4">
		<p class="error">Cannot reach the server: {loadError}</p>
	</div>
{:else if session.loaded && !target}
	{@render children()}
{/if}
