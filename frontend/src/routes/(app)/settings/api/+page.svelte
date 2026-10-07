<script lang="ts">
	import { store, loadModels } from '#lib/chats.svelte.ts';
	import CopyButton from '#lib/components/CopyButton.svelte';

	if (!store.modelsLoaded) loadModels();
	const origin = location.origin;
	const model = $derived(store.models[0]?.name ?? 'openai/gpt-4o');

	const examples = $derived([
		{
			title: 'curl (OpenAI format)',
			code: `curl ${origin}/api/v1/chat/completions \\
  -H "Authorization: Bearer $BRAID_API_KEY" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "${model}", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'`
		},
		{
			title: 'Python, OpenAI SDK',
			code: `from openai import OpenAI

client = OpenAI(base_url="${origin}/api/v1", api_key="sk-braid-...")
stream = client.chat.completions.create(
    model="${model}",
    messages=[{"role": "user", "content": "Hello"}],
    stream=True,
)
for chunk in stream:
    print(chunk.choices[0].delta.content or "", end="")`
		},
		{
			title: 'Python, Anthropic SDK',
			code: `import anthropic

client = anthropic.Anthropic(base_url="${origin}/api/anthropic", api_key="sk-braid-...")
with client.messages.stream(
    model="${model}",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello"}],
) as stream:
    for text in stream.text_stream:
        print(text, end="")`
		},
		{ title: 'List your models', code: `curl ${origin}/api/v1/models -H "Authorization: Bearer $BRAID_API_KEY"` },
		{ title: 'Your usage by day and model', code: `curl ${origin}/api/v1/usage -H "Authorization: Bearer $BRAID_API_KEY"` }
	]);
</script>

<div class="h-full overflow-y-auto">
	<div class="mx-auto max-w-3xl space-y-6 px-4 py-6">
		<a href="/settings" class="text-sm text-zinc-500 hover:underline">← Settings</a>
		<h1 class="text-2xl font-semibold">Using the API</h1>
		<p>One key reaches every model you can use, whichever provider serves it. Any model works through either format.</p>
		<div class="card space-y-2 text-sm">
			<div><span class="font-medium">OpenAI-compatible base URL:</span> <code>{origin}/api/v1</code> <CopyButton text="{origin}/api/v1" /></div>
			<div><span class="font-medium">Anthropic-compatible base URL:</span> <code>{origin}/api/anthropic</code> <CopyButton text="{origin}/api/anthropic" /></div>
			<div><span class="font-medium">Auth:</span> <code>Authorization: Bearer sk-braid-…</code> or <code>x-api-key</code></div>
			<div><span class="font-medium">Model names:</span> {#each store.models as m, i}<code>{m.name}</code>{i < store.models.length - 1 ? ', ' : ''}{:else}none yet{/each}</div>
			<div class="muted">Responses carry <code>x-braid-provider</code> and <code>x-braid-model</code> headers. Errors use the error format of the endpoint you called.</div>
		</div>
		{#each examples as ex}
			<section>
				<div class="flex items-center justify-between"><h2 class="font-semibold">{ex.title}</h2><CopyButton text={ex.code} /></div>
				<pre class="mt-2 overflow-x-auto rounded-lg bg-zinc-100 p-3 text-[13px] dark:bg-zinc-900"><code>{ex.code}</code></pre>
			</section>
		{/each}
	</div>
</div>
