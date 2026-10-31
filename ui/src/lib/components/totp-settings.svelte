<script lang="ts">
	import { onMount } from 'svelte';
	import { REGEXP_ONLY_DIGITS } from 'bits-ui';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '#lib/api.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as InputOTP from '#lib/components/ui/input-otp/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import type { TotpEnrollment, TotpStatus } from '#lib/types.js';

	let status = $state<TotpStatus | null>(null);
	let enrollment = $state<TotpEnrollment | null>(null);
	let code = $state('');
	let busy = $state(false);

	onMount(async () => {
		try {
			status = await api.totpStatus();
		} catch (e) {
			toast.error(errorMessage(e));
		}
	});

	async function run(action: () => Promise<void>) {
		busy = true;
		try {
			await action();
		} catch (e) {
			code = '';
			toast.error(errorMessage(e));
		} finally {
			busy = false;
		}
	}

	const startEnrollment = () =>
		run(async () => {
			enrollment = await api.totpEnroll();
			status = { enabled: false, pending: true };
			code = '';
		});

	const confirmEnrollment = () =>
		run(async () => {
			status = await api.totpVerify(code);
			enrollment = null;
			code = '';
			toast.success('Two-factor authentication is now enabled.');
		});

	const cancelEnrollment = () =>
		run(async () => {
			status = await api.totpDisable('');
			enrollment = null;
			code = '';
		});

	const disable = () =>
		run(async () => {
			status = await api.totpDisable(code);
			code = '';
			toast.success('Two-factor authentication has been disabled.');
		});

	async function copySecret() {
		if (!enrollment) return;
		try {
			await navigator.clipboard.writeText(enrollment.secret);
			toast.success('Secret copied.');
		} catch {
			toast.error('Copying failed; select the secret and copy it manually.');
		}
	}
</script>

{#snippet codeInput(id: string)}
	<InputOTP.Root
		{id}
		maxlength={6}
		pattern={REGEXP_ONLY_DIGITS}
		autocomplete="one-time-code"
		bind:value={code}
	>
		{#snippet children({ cells })}
			<InputOTP.Group>
				{#each cells as cell, i (i)}
					<InputOTP.Slot {cell} />
				{/each}
			</InputOTP.Group>
		{/snippet}
	</InputOTP.Root>
{/snippet}

<Card.Root>
	<Card.Header>
		<Card.Title class="flex items-center gap-2">
			Two-factor authentication
			{#if status?.enabled}<Badge>Enabled</Badge>{:else if status}<Badge variant="outline"
					>Off</Badge
				>{/if}
		</Card.Title>
		<Card.Description>
			Require a time-based one-time code from an authenticator app when signing in.
		</Card.Description>
	</Card.Header>
	<Card.Content>
		{#if !status}
			<Skeleton class="h-10 w-40" />
		{:else if status.enabled}
			<form
				class="grid max-w-sm gap-3"
				onsubmit={(e) => {
					e.preventDefault();
					void disable();
				}}
			>
				<Label for="totp-disable">Enter a current code to disable</Label>
				{@render codeInput('totp-disable')}
				<div>
					<Button type="submit" variant="destructive" disabled={busy || code.length !== 6}>
						Disable two-factor authentication
					</Button>
				</div>
			</form>
		{:else if enrollment}
			<div class="grid gap-6 md:grid-cols-[auto_1fr]">
				<img
					src={enrollment.qr_code}
					alt="QR code for your authenticator app"
					class="size-48 rounded-md border bg-white p-2"
				/>
				<form
					class="grid content-start gap-3"
					onsubmit={(e) => {
						e.preventDefault();
						void confirmEnrollment();
					}}
				>
					<p class="text-sm">
						Scan the QR code with your authenticator app, or enter this secret manually:
					</p>
					<div class="flex flex-wrap items-center gap-2">
						<code class="rounded bg-muted px-2 py-1 text-sm break-all">{enrollment.secret}</code>
						<Button variant="outline" size="sm" onclick={copySecret}>Copy</Button>
					</div>
					<Label for="totp-confirm">Then enter the 6-digit code it shows</Label>
					{@render codeInput('totp-confirm')}
					<div class="flex gap-2">
						<Button type="submit" disabled={busy || code.length !== 6}>Confirm</Button>
						<Button variant="ghost" onclick={cancelEnrollment} disabled={busy}>Cancel</Button>
					</div>
				</form>
			</div>
		{:else}
			{#if status.pending}
				<p class="mb-3 text-sm text-muted-foreground">
					A previous setup was not completed. Start again to get a new secret.
				</p>
			{/if}
			<Button onclick={startEnrollment} disabled={busy}>Set up two-factor authentication</Button>
		{/if}
	</Card.Content>
</Card.Root>
