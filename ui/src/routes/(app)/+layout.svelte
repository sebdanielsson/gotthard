<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { toggleMode } from 'mode-watcher';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import MoonIcon from '@lucide/svelte/icons/moon';
	import SunIcon from '@lucide/svelte/icons/sun';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '#lib/api.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { cn } from '#lib/utils.js';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const nav = [
		{ path: '', label: 'Dashboard' },
		{ path: 'firewall', label: 'Firewall' },
		{ path: 'settings', label: 'Settings' }
	] as const;

	async function logout() {
		try {
			await api.logout();
		} catch (e) {
			toast.error(errorMessage(e));
		}
		await goto(resolve('/login'), { invalidateAll: true });
	}
</script>

<div class="min-h-svh">
	<header class="border-b">
		<div class="mx-auto flex max-w-6xl flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3">
			<a href={resolve('')} class="font-semibold tracking-tight">Gotthard</a>
			<nav class="flex gap-1">
				{#each nav as item (item.path)}
					<Button
						href={resolve(item.path)}
						variant="ghost"
						size="sm"
						class={cn(page.url.pathname === resolve(item.path) && 'bg-muted')}
						aria-current={page.url.pathname === resolve(item.path) ? 'page' : undefined}
					>
						{item.label}
					</Button>
				{/each}
			</nav>
			<div class="ml-auto flex items-center gap-2">
				<span class="text-sm text-muted-foreground">{data.me.username}</span>
				<Button variant="ghost" size="icon-sm" onclick={toggleMode} aria-label="Toggle dark mode">
					<SunIcon class="scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" />
					<MoonIcon
						class="absolute scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0"
					/>
				</Button>
				<Button variant="outline" size="sm" onclick={logout}>
					<LogOutIcon />
					Sign out
				</Button>
			</div>
		</div>
	</header>
	<main class="mx-auto max-w-6xl px-4 py-6">
		{@render children()}
	</main>
</div>
