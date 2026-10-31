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
	import { parseRuleset, type NftTable } from '#lib/nft.js';
	import type { NftRuleset } from '#lib/types.js';

	let raw = $state<NftRuleset | null>(null);
	let error = $state<string | null>(null);
	let loading = $state(false);
	const tables = $derived<NftTable[]>(raw ? parseRuleset(raw) : []);

	async function refresh() {
		loading = true;
		try {
			raw = await api.ruleset();
			error = null;
		} catch (e) {
			error = errorMessage(e);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
	});
</script>

<div class="mb-6 flex items-center justify-between gap-4">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Firewall</h1>
		<p class="text-sm text-muted-foreground">Active nftables ruleset (read-only).</p>
	</div>
	<Button variant="outline" size="sm" onclick={refresh} disabled={loading}>
		<RefreshCwIcon class={loading ? 'animate-spin' : ''} />
		Refresh
	</Button>
</div>

{#if error}
	<Alert.Root variant="destructive">
		<TriangleAlertIcon />
		<Alert.Title>Cannot read the ruleset</Alert.Title>
		<Alert.Description>{error}</Alert.Description>
	</Alert.Root>
{:else if !raw}
	<Skeleton class="h-48" />
{:else if tables.length === 0}
	<Card.Root>
		<Card.Content class="text-muted-foreground">The ruleset is empty.</Card.Content>
	</Card.Root>
{:else}
	<div class="grid gap-6">
		{#each tables as table (`${table.family} ${table.name}`)}
			<Card.Root>
				<Card.Header>
					<Card.Title class="font-mono">table {table.family} {table.name}</Card.Title>
					{#if table.sets.length}
						<Card.Description class="flex flex-wrap gap-2">
							{#each table.sets as set (set.name)}
								<Badge variant="outline" class="font-mono">
									{set.kind} @{set.name}: {set.type} ({set.elements})
								</Badge>
							{/each}
						</Card.Description>
					{/if}
				</Card.Header>
				<Card.Content class="grid gap-6">
					{#each table.chains as chain (chain.name)}
						<section>
							<div class="mb-2 flex flex-wrap items-center gap-2">
								<h3 class="font-mono font-medium">chain {chain.name}</h3>
								{#if chain.type}<Badge variant="secondary">type {chain.type}</Badge>{/if}
								{#if chain.hook}<Badge variant="secondary">hook {chain.hook}</Badge>{/if}
								{#if chain.prio !== undefined}
									<Badge variant="secondary">priority {chain.prio}</Badge>
								{/if}
								{#if chain.policy}
									<Badge variant={chain.policy === 'drop' ? 'destructive' : 'default'}>
										policy {chain.policy}
									</Badge>
								{/if}
							</div>
							{#if chain.rules.length}
								<Table.Root>
									<Table.Header>
										<Table.Row>
											<Table.Head class="w-16">Handle</Table.Head>
											<Table.Head>Rule</Table.Head>
										</Table.Row>
									</Table.Header>
									<Table.Body>
										{#each chain.rules as rule (rule.handle)}
											<Table.Row>
												<Table.Cell class="align-top font-mono text-xs text-muted-foreground">
													{rule.handle}
												</Table.Cell>
												<Table.Cell class="font-mono text-xs break-all whitespace-normal">
													{rule.text}
													{#if rule.comment}
														<span class="text-muted-foreground">comment "{rule.comment}"</span>
													{/if}
												</Table.Cell>
											</Table.Row>
										{/each}
									</Table.Body>
								</Table.Root>
							{:else}
								<p class="text-sm text-muted-foreground">No rules.</p>
							{/if}
						</section>
					{/each}
				</Card.Content>
			</Card.Root>
		{/each}

		<details class="rounded-lg border p-4">
			<summary class="cursor-pointer text-sm font-medium">Raw JSON</summary>
			<pre class="mt-3 max-h-[32rem] overflow-auto text-xs">{JSON.stringify(raw, null, 2)}</pre>
		</details>
	</div>
{/if}
