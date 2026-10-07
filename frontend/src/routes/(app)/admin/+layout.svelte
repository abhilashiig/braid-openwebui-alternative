<script lang="ts">
	import { page } from '$app/state';

	let { children } = $props();

	const tabs = [
		['/admin/providers', 'Providers'],
		['/admin/models', 'Models'],
		['/admin/users', 'Users'],
		['/admin/groups', 'Groups'],
		['/admin/skills', 'Skills'],
		['/admin/usage', 'Usage'],
		['/admin/audit', 'Audit log'],
		['/admin/settings', 'Settings']
	];
</script>

<div class="h-full overflow-y-auto">
	<div class="mx-auto max-w-6xl px-4 py-6">
		<h1 class="text-2xl font-semibold">Admin</h1>
		<nav class="mt-4 flex gap-1 overflow-x-auto border-b border-zinc-200 dark:border-zinc-800" aria-label="Admin sections">
			{#each tabs as [href, label]}
				<a
					{href}
					class="-mb-px border-b-2 px-3 py-2 text-sm whitespace-nowrap {page.url.pathname.startsWith(href)
						? 'border-zinc-900 font-medium dark:border-zinc-100'
						: 'border-transparent text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100'}"
					aria-current={page.url.pathname.startsWith(href) ? 'page' : undefined}>{label}</a
				>
			{/each}
		</nav>
		<div class="py-6">{@render children()}</div>
	</div>
</div>
