<script lang="ts">
	import ApiKeySettings from '#lib/components/api-key-settings.svelte';
	import TotpSettings from '#lib/components/totp-settings.svelte';
</script>

<h1 class="mb-6 text-2xl font-semibold tracking-tight">Settings</h1>

<div class="grid gap-6">
	<TotpSettings />
	<ApiKeySettings />
</div>
