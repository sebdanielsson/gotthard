<script lang="ts">
	import { onMount } from 'svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { api, errorMessage } from '#lib/api.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { formatBytes, formatUptime } from '#lib/format.js';
	import type { IpLink, SystemInfo } from '#lib/types.js';

	let system = $state<SystemInfo | null>(null);
	let systemError = $state<string | null>(null);
	let links = $state<IpLink[] | null>(null);
	let linksError = $state<string | null>(null);
	let loading = $state(false);

	async function refresh() {
		loading = true;
		const [sys, ifs] = await Promise.allSettled([api.system(), api.interfaces()]);
		if (sys.status === 'fulfilled') {
			system = sys.value;
			systemError = null;
		} else {
			systemError = errorMessage(sys.reason);
		}
		if (ifs.status === 'fulfilled') {
			links = ifs.value.interfaces;
			linksError = null;
		} else {
			linksError = errorMessage(ifs.reason);
		}
		loading = false;
	}

	onMount(() => {
		void refresh();
		const timer = setInterval(() => void refresh(), 15_000);
		return () => clearInterval(timer);
	});

	const memory = $derived(system?.memory ?? null);
	const memoryUsedPct = $derived(
		memory && memory.total_bytes > 0
			? Math.round((1 - memory.available_bytes / memory.total_bytes) * 100)
			: null
	);
	const osName = $derived(system?.os_release?.PRETTY_NAME ?? system?.os_release?.NAME ?? null);

	function stateVariant(state: string): 'default' | 'secondary' | 'destructive' | 'outline' {
		if (state === 'UP') return 'default';
		if (state === 'DOWN') return 'destructive';
		return 'outline';
	}
</script>

<div class="mb-6 flex items-center justify-between gap-4">
	<h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
	<Button variant="outline" size="sm" onclick={refresh} disabled={loading}>
		<RefreshCwIcon class={loading ? 'animate-spin' : ''} />
		Refresh
	</Button>
</div>

<div class="grid gap-6">
	<Card.Root>
		<Card.Header>
			<Card.Title>System</Card.Title>
			<Card.Description>{system?.hostname ?? '—'}</Card.Description>
		</Card.Header>
		<Card.Content>
			{#if systemError}
				<Alert.Root variant="destructive">
					<TriangleAlertIcon />
					<Alert.Title>Cannot load system information</Alert.Title>
					<Alert.Description>{systemError}</Alert.Description>
				</Alert.Root>
			{:else if !system}
				<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
					{#each { length: 6 }, i (i)}
						<Skeleton class="h-12" />
					{/each}
				</div>
			{:else}
				<dl class="grid gap-x-6 gap-y-4 sm:grid-cols-2 lg:grid-cols-3">
					<div>
						<dt class="text-sm text-muted-foreground">Operating system</dt>
						<dd class="font-medium">{osName ?? 'unknown'}</dd>
					</div>
					<div>
						<dt class="text-sm text-muted-foreground">Kernel</dt>
						<dd class="font-mono text-sm">{system.kernel_release ?? 'unknown'}</dd>
					</div>
					<div>
						<dt class="text-sm text-muted-foreground">Uptime</dt>
						<dd class="font-medium">
							{system.uptime_seconds === null ? 'unknown' : formatUptime(system.uptime_seconds)}
						</dd>
					</div>
					<div>
						<dt class="text-sm text-muted-foreground">Load average (1 / 5 / 15 min)</dt>
						<dd class="font-mono text-sm">
							{#if system.load_average}
								{system.load_average.one.toFixed(2)} / {system.load_average.five.toFixed(2)} /
								{system.load_average.fifteen.toFixed(2)}
							{:else}
								unknown
							{/if}
						</dd>
					</div>
					<div class="sm:col-span-2">
						<dt class="text-sm text-muted-foreground">Memory</dt>
						<dd>
							{#if memory && memoryUsedPct !== null}
								<div class="flex items-baseline justify-between text-sm">
									<span class="font-medium">
										{formatBytes(memory.total_bytes - memory.available_bytes)} used
									</span>
									<span class="text-muted-foreground">
										of {formatBytes(memory.total_bytes)} ({memoryUsedPct}%)
									</span>
								</div>
								<div
									class="mt-1 h-2 overflow-hidden rounded-full bg-muted"
									role="meter"
									aria-label="Memory used"
									aria-valuemin={0}
									aria-valuemax={100}
									aria-valuenow={memoryUsedPct}
								>
									<div class="h-full bg-primary" style:width="{memoryUsedPct}%"></div>
								</div>
							{:else}
								unknown
							{/if}
						</dd>
					</div>
				</dl>
			{/if}
		</Card.Content>
		{#if system}
			<Card.Footer class="text-xs text-muted-foreground">
				gotthard-web {system.web_version}
			</Card.Footer>
		{/if}
	</Card.Root>

	<Card.Root>
		<Card.Header>
			<Card.Title>Interfaces</Card.Title>
			<Card.Description>Links and addresses as reported by <code>ip addr</code>.</Card.Description>
		</Card.Header>
		<Card.Content>
			{#if linksError}
				<Alert.Root variant="destructive">
					<TriangleAlertIcon />
					<Alert.Title>Cannot list interfaces</Alert.Title>
					<Alert.Description>{linksError}</Alert.Description>
				</Alert.Root>
			{:else if !links}
				<Skeleton class="h-32" />
			{:else}
				<Table.Root>
					<Table.Header>
						<Table.Row>
							<Table.Head>Interface</Table.Head>
							<Table.Head>State</Table.Head>
							<Table.Head>MAC address</Table.Head>
							<Table.Head class="text-right">MTU</Table.Head>
							<Table.Head>Addresses</Table.Head>
						</Table.Row>
					</Table.Header>
					<Table.Body>
						{#each links as link (link.ifindex)}
							<Table.Row>
								<Table.Cell class="font-medium">
									{link.ifname}
									{#if link.master}
										<span class="text-xs text-muted-foreground">→ {link.master}</span>
									{/if}
								</Table.Cell>
								<Table.Cell>
									<Badge variant={stateVariant(link.operstate)}>{link.operstate}</Badge>
								</Table.Cell>
								<Table.Cell class="font-mono text-xs">{link.address ?? '—'}</Table.Cell>
								<Table.Cell class="text-right font-mono text-xs">{link.mtu}</Table.Cell>
								<Table.Cell class="font-mono text-xs">
									{#each link.addr_info as addr (`${addr.local}/${addr.prefixlen}`)}
										<div>
											{addr.local}/{addr.prefixlen}
											{#if addr.dynamic}<span class="text-muted-foreground">dynamic</span>{/if}
										</div>
									{:else}
										<span class="text-muted-foreground">—</span>
									{/each}
								</Table.Cell>
							</Table.Row>
						{:else}
							<Table.Row>
								<Table.Cell colspan={5} class="text-center text-muted-foreground">
									No interfaces.
								</Table.Cell>
							</Table.Row>
						{/each}
					</Table.Body>
				</Table.Root>
			{/if}
		</Card.Content>
	</Card.Root>
</div>
