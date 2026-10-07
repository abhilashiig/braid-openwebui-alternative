<script lang="ts">
	let { password = $bindable(''), confirm = $bindable('') }: { password: string; confirm: string } = $props();
</script>

<div>
	<label class="label" for="password">Password</label>
	<input id="password" class="input" type="password" autocomplete="new-password" minlength="12" required bind:value={password} aria-describedby="pw-hint" />
	<p id="pw-hint" class="muted mt-1">At least 12 characters. Passwords found in known breaches are rejected.</p>
</div>
<div>
	<label class="label" for="confirm">Confirm password</label>
	<input id="confirm" class="input" type="password" autocomplete="new-password" required bind:value={confirm} />
	{#if confirm && confirm !== password}<p class="mt-1 text-sm text-red-600">Passwords don't match</p>{/if}
</div>
