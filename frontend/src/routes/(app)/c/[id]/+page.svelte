<script lang="ts">
	import { page } from '$app/state';
	import ChatView from '#lib/components/ChatView.svelte';
</script>

{#key page.params.id}
	<ChatView chatId={page.params.id} />
{/key}
