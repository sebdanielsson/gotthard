<script lang="ts">
	import { onMount } from 'svelte';
	import TrashIcon from '@lucide/svelte/icons/trash-2';
	import { toast } from 'svelte-sonner';
	import { api, errorMessage } from '#lib/api.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { formatTimestamp } from '#lib/format.js';
	import type { ApiKey, ApiKeyList, NewApiKey, Scope } from '#lib/types.js';

	const SCOPE_LABELS: Record<Scope, string> = {
		'system:read': 'Read system information',
		'network:read': 'Read interfaces',
		'firewall:read': 'Read firewall ruleset'
	};

	let list = $state<ApiKeyList | null>(null);
	let name = $state('');
	let selected = $state<Scope[]>([]);
	let busy = $state(false);
	let created = $state<NewApiKey | null>(null);
	let toDelete = $state<ApiKey | null>(null);

	async function load() {
		try {
			list = await api.listKeys();
		} catch (e) {
			toast.error(errorMessage(e));
		}
	}

	onMount(load);

	function toggleScope(scope: Scope, checked: boolean) {
		selected = checked ? [...selected, scope] : selected.filter((s) => s !== scope);
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		try {
			created = await api.createKey(name.trim(), selected);
			name = '';
			selected = [];
			await load();
		} catch (e) {
			toast.error(errorMessage(e));
		} finally {
			busy = false;
		}
	}

	async function confirmDelete() {
		if (!toDelete) return;
		const key = toDelete;
		busy = true;
		try {
			await api.deleteKey(key.id);
			toast.success(`Deleted “${key.name}”.`);
			toDelete = null;
			await load();
		} catch (e) {
			toast.error(errorMessage(e));
		} finally {
			busy = false;
		}
	}

	async function copyToken() {
		if (!created) return;
		try {
			await navigator.clipboard.writeText(created.token);
			toast.success('Token copied.');
		} catch {
			toast.error('Copying failed; select the token and copy it manually.');
		}
	}
</script>

<Card.Root>
	<Card.Header>
		<Card.Title>API keys</Card.Title>
		<Card.Description>
			Tokens for scripts and automation, sent as <code>Authorization: Bearer &lt;token&gt;</code>.
			Keys are read-only and cannot manage other keys or two-factor settings.
		</Card.Description>
	</Card.Header>
	<Card.Content class="grid gap-6">
		{#if !list}
			<Skeleton class="h-24" />
		{:else}
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head>Name</Table.Head>
						<Table.Head>Scopes</Table.Head>
						<Table.Head>Created</Table.Head>
						<Table.Head class="w-12"><span class="sr-only">Actions</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each list.keys as key (key.id)}
						<Table.Row>
							<Table.Cell>
								<div class="font-medium">{key.name}</div>
								<div class="font-mono text-xs text-muted-foreground">{key.id}</div>
							</Table.Cell>
							<Table.Cell>
								<div class="flex flex-wrap gap-1">
									{#each key.scopes as scope (scope)}
										<Badge variant="secondary" class="font-mono">{scope}</Badge>
									{/each}
								</div>
							</Table.Cell>
							<Table.Cell class="text-sm">
								{formatTimestamp(key.created_at)}
								<div class="text-xs text-muted-foreground">by {key.created_by}</div>
							</Table.Cell>
							<Table.Cell>
								<Button
									variant="ghost"
									size="icon-sm"
									aria-label={`Delete ${key.name}`}
									onclick={() => (toDelete = key)}
								>
									<TrashIcon />
								</Button>
							</Table.Cell>
						</Table.Row>
					{:else}
						<Table.Row>
							<Table.Cell colspan={4} class="text-center text-muted-foreground">
								No API keys yet.
							</Table.Cell>
						</Table.Row>
					{/each}
				</Table.Body>
			</Table.Root>

			<form class="grid max-w-md gap-4" onsubmit={create}>
				<div class="grid gap-2">
					<Label for="key-name">New key name</Label>
					<Input id="key-name" maxlength={64} placeholder="e.g. monitoring" bind:value={name} />
				</div>
				<fieldset class="grid gap-2">
					<legend class="mb-1 text-sm font-medium">Scopes</legend>
					{#each list.available_scopes as scope (scope)}
						<div class="flex items-center gap-2">
							<Checkbox
								id={`scope-${scope}`}
								checked={selected.includes(scope)}
								onCheckedChange={(checked) => toggleScope(scope, checked)}
							/>
							<Label for={`scope-${scope}`} class="font-normal">
								{SCOPE_LABELS[scope] ?? scope}
								<span class="font-mono text-xs text-muted-foreground">{scope}</span>
							</Label>
						</div>
					{/each}
				</fieldset>
				<div>
					<Button type="submit" disabled={busy || !name.trim() || selected.length === 0}>
						Create key
					</Button>
				</div>
			</form>
		{/if}
	</Card.Content>
</Card.Root>

<Dialog.Root
	open={created !== null}
	onOpenChange={(open) => {
		if (!open) created = null;
	}}
>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>API key created</Dialog.Title>
			<Dialog.Description>
				Copy the token now. It is stored only as a hash and will not be shown again.
			</Dialog.Description>
		</Dialog.Header>
		{#if created}
			<code class="rounded bg-muted p-3 font-mono text-xs break-all select-all"
				>{created.token}</code
			>
		{/if}
		<Dialog.Footer>
			<Button variant="outline" onclick={copyToken}>Copy</Button>
			<Button onclick={() => (created = null)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root
	open={toDelete !== null}
	onOpenChange={(open) => {
		if (!open) toDelete = null;
	}}
>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>Delete API key?</Dialog.Title>
			<Dialog.Description>
				“{toDelete?.name}” stops working immediately. This cannot be undone.
			</Dialog.Description>
		</Dialog.Header>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (toDelete = null)}>Cancel</Button>
			<Button variant="destructive" onclick={confirmDelete} disabled={busy}>Delete</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
