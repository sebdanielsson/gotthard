<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { REGEXP_ONLY_DIGITS } from 'bits-ui';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { ApiError, api, errorMessage } from '#lib/api.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as InputOTP from '#lib/components/ui/input-otp/index.js';
	import { Label } from '#lib/components/ui/label/index.js';

	let step = $state<'password' | 'totp'>('password');
	let username = $state('');
	let password = $state('');
	let code = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);

	async function submitPassword(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		try {
			const res = await api.login(username, password);
			password = '';
			if (res.status === 'totp_required') {
				step = 'totp';
			} else {
				await goto(resolve(''), { invalidateAll: true });
			}
		} catch (e) {
			error = errorMessage(e);
		} finally {
			busy = false;
		}
	}

	async function submitCode(event?: SubmitEvent) {
		event?.preventDefault();
		if (code.length !== 6 || busy) return;
		busy = true;
		error = null;
		try {
			await api.loginTotp(code);
			await goto(resolve(''), { invalidateAll: true });
		} catch (e) {
			code = '';
			error = errorMessage(e);
			if (e instanceof ApiError && e.code === 'no_pending_login') {
				step = 'password';
			}
		} finally {
			busy = false;
		}
	}

	function backToPassword() {
		step = 'password';
		code = '';
		error = null;
	}
</script>

<main class="flex min-h-svh items-center justify-center p-4">
	<Card.Root class="w-full max-w-sm">
		<Card.Header>
			<Card.Title class="text-xl">Gotthard</Card.Title>
			<Card.Description>
				{step === 'password'
					? 'Sign in with a router account in the wheel group.'
					: 'Enter the 6-digit code from your authenticator app.'}
			</Card.Description>
		</Card.Header>
		<Card.Content>
			{#if error}
				<Alert.Root variant="destructive" class="mb-4">
					<TriangleAlertIcon />
					<Alert.Title>Sign-in failed</Alert.Title>
					<Alert.Description>{error}</Alert.Description>
				</Alert.Root>
			{/if}

			{#if step === 'password'}
				<form class="grid gap-4" onsubmit={submitPassword}>
					<div class="grid gap-2">
						<Label for="username">Username</Label>
						<Input
							id="username"
							autocomplete="username"
							autocapitalize="none"
							spellcheck={false}
							required
							bind:value={username}
						/>
					</div>
					<div class="grid gap-2">
						<Label for="password">Password</Label>
						<Input
							id="password"
							type="password"
							autocomplete="current-password"
							required
							bind:value={password}
						/>
					</div>
					<Button type="submit" disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</Button>
				</form>
			{:else}
				<form class="grid gap-4" onsubmit={submitCode}>
					<div class="flex justify-center">
						<InputOTP.Root
							maxlength={6}
							pattern={REGEXP_ONLY_DIGITS}
							autocomplete="one-time-code"
							bind:value={code}
							onComplete={() => submitCode()}
						>
							{#snippet children({ cells })}
								<InputOTP.Group>
									{#each cells as cell, i (i)}
										<InputOTP.Slot {cell} />
									{/each}
								</InputOTP.Group>
							{/snippet}
						</InputOTP.Root>
					</div>
					<Button type="submit" disabled={busy || code.length !== 6}>
						{busy ? 'Verifying…' : 'Verify'}
					</Button>
					<Button variant="ghost" onclick={backToPassword} disabled={busy}>Back</Button>
				</form>
			{/if}
		</Card.Content>
	</Card.Root>
</main>
