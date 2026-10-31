// Turns `nft -j list ruleset` output into a table/chain/rule tree and renders
// rule expressions roughly the way `nft list ruleset` prints them. Anything
// unrecognised falls back to compact JSON, so nothing is ever hidden.

import type { NftRuleset } from './types';

export interface NftRule {
	handle: number;
	text: string;
	comment?: string;
}

export interface NftChain {
	name: string;
	type?: string;
	hook?: string;
	prio?: number | string;
	policy?: string;
	rules: NftRule[];
}

export interface NftSet {
	name: string;
	kind: 'set' | 'map';
	type: string;
	elements: number;
}

export interface NftTable {
	family: string;
	name: string;
	chains: NftChain[];
	sets: NftSet[];
}

type Json = unknown;
type Obj = Record<string, Json>;

const isObj = (v: Json): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: Json): string | undefined => (typeof v === 'string' ? v : undefined);

export function parseRuleset(ruleset: NftRuleset): NftTable[] {
	const tables = new Map<string, NftTable>();
	const key = (family: Json, name: Json) => `${String(family)}\u0000${String(name)}`;
	const tableFor = (o: Obj) => tables.get(key(o.family, o.table));

	for (const entry of ruleset.nftables) {
		if (isObj(entry.table)) {
			const t = entry.table;
			tables.set(key(t.family, t.name), {
				family: String(t.family),
				name: String(t.name),
				chains: [],
				sets: []
			});
		} else if (isObj(entry.chain)) {
			const c = entry.chain;
			tableFor(c)?.chains.push({
				name: String(c.name),
				type: str(c.type),
				hook: str(c.hook),
				prio: typeof c.prio === 'number' || typeof c.prio === 'string' ? c.prio : undefined,
				policy: str(c.policy),
				rules: []
			});
		} else if (isObj(entry.rule)) {
			const r = entry.rule;
			const chain = tableFor(r)?.chains.find((c) => c.name === r.chain);
			const exprs = Array.isArray(r.expr) ? r.expr : [];
			chain?.rules.push({
				handle: Number(r.handle),
				text: exprs.map(formatStatement).join(' '),
				comment: str(r.comment)
			});
		} else if (isObj(entry.set) || isObj(entry.map)) {
			const kind = isObj(entry.set) ? 'set' : 'map';
			const s = (entry.set ?? entry.map) as Obj;
			tableFor(s)?.sets.push({
				name: String(s.name),
				kind,
				type: Array.isArray(s.type) ? s.type.join(' . ') : String(s.type),
				elements: Array.isArray(s.elem) ? s.elem.length : 0
			});
		}
	}
	return [...tables.values()];
}

const VERDICTS = new Set(['accept', 'drop', 'continue', 'return', 'queue']);
const BARE_META = new Set([
	'iif',
	'oif',
	'iifname',
	'oifname',
	'iiftype',
	'oiftype',
	'iifgroup',
	'oifgroup',
	'skuid',
	'skgid',
	'mark',
	'pkttype',
	'nftrace',
	'cpu',
	'cgroup'
]);
const BINARY_OPS = ['&', '|', '^', '<<', '>>'];

export function formatStatement(stmt: Json): string {
	if (!isObj(stmt)) return JSON.stringify(stmt);
	const [name, arg] = Object.entries(stmt)[0] ?? [];
	if (name === undefined) return '';
	if (VERDICTS.has(name)) return name;

	const a = isObj(arg) ? arg : {};
	switch (name) {
		case 'match': {
			const op = str(a.op) ?? '==';
			const left = formatValue(a.left);
			const right = formatValue(a.right);
			return op === '==' || op === 'in' ? `${left} ${right}` : `${left} ${op} ${right}`;
		}
		case 'jump':
		case 'goto':
			return `${name} ${String(a.target)}`;
		case 'counter':
			return isObj(arg) ? `counter packets ${a.packets} bytes ${a.bytes}` : 'counter';
		case 'log': {
			const parts = ['log'];
			if (a.prefix !== undefined) parts.push(`prefix ${JSON.stringify(a.prefix)}`);
			if (a.level !== undefined) parts.push(`level ${String(a.level)}`);
			return parts.join(' ');
		}
		case 'reject':
			return a.type
				? `reject with ${String(a.type)} ${formatValue(a.expr ?? '')}`.trim()
				: 'reject';
		case 'masquerade':
		case 'notrack':
			return name;
		case 'snat':
		case 'dnat':
		case 'redirect': {
			const target = [a.addr, a.port]
				.filter((v) => v !== undefined)
				.map(formatValue)
				.join(':');
			return target ? `${name} to ${target}` : name;
		}
		case 'limit':
			return `limit rate ${a.inv ? 'over ' : ''}${String(a.rate)}/${String(a.per ?? 'second')}`;
		case 'mangle':
			return `${formatValue(a.key)} set ${formatValue(a.value)}`;
		default:
			return JSON.stringify(stmt);
	}
}

export function formatValue(value: Json): string {
	if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
		return String(value);
	}
	if (Array.isArray(value)) return value.map(formatValue).join(',');
	if (!isObj(value)) return JSON.stringify(value);

	const [name, arg] = Object.entries(value)[0] ?? [];
	const a = isObj(arg) ? arg : {};
	switch (name) {
		case 'payload':
			return a.protocol !== undefined
				? `${String(a.protocol)} ${String(a.field)}`
				: `@${String(a.base)},${String(a.offset)},${String(a.len)}`;
		case 'meta': {
			const key = String(a.key);
			return BARE_META.has(key) ? key : `meta ${key}`;
		}
		case 'ct':
			return ['ct', a.dir, a.key].filter((v) => v !== undefined).join(' ');
		case 'set':
			return `{ ${(Array.isArray(arg) ? arg : [arg]).map(formatValue).join(', ')} }`;
		case 'range':
			return Array.isArray(arg) ? arg.map(formatValue).join('-') : JSON.stringify(arg);
		case 'prefix':
			return `${formatValue(a.addr)}/${String(a.len)}`;
		case 'concat':
			return Array.isArray(arg) ? arg.map(formatValue).join(' . ') : JSON.stringify(arg);
		case 'elem':
			return formatValue(a.val);
		case 'map':
			return `${formatValue(a.key)} map ${formatValue(a.data)}`;
		case 'fib':
			return `fib ${(Array.isArray(a.flags) ? a.flags : [a.flags]).join(' . ')} ${String(a.result)}`;
		default:
			if (name !== undefined && BINARY_OPS.includes(name) && Array.isArray(arg)) {
				return arg.map(formatValue).join(` ${name} `);
			}
			return JSON.stringify(value);
	}
}
