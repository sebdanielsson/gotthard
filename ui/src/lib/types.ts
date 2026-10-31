// Types mirroring the gotthard-web JSON API (see web/README.md).

export type Scope = 'system:read' | 'network:read' | 'firewall:read';

export interface Me {
	username: string;
	auth_method: 'session' | 'api_key';
	scopes: Scope[];
	totp_enabled?: boolean;
	api_key?: { id: string; name: string };
}

export type LoginResponse = { status: 'ok'; user: Me } | { status: 'totp_required' };

export interface SystemInfo {
	hostname: string | null;
	kernel_release: string | null;
	uptime_seconds: number | null;
	load_average: { one: number; five: number; fifteen: number } | null;
	memory: {
		total_bytes: number;
		available_bytes: number;
		free_bytes: number;
		buffers_bytes: number;
		cached_bytes: number;
		swap_total_bytes: number;
		swap_free_bytes: number;
	} | null;
	os_release: Record<string, string> | null;
	web_version: string;
}

/** One entry of iproute2's `ip -j addr show` (only the fields the UI uses). */
export interface IpLink {
	ifindex: number;
	ifname: string;
	flags: string[];
	mtu: number;
	operstate: string;
	link_type: string;
	address?: string;
	master?: string;
	addr_info: {
		family: 'inet' | 'inet6' | string;
		local: string;
		prefixlen: number;
		scope: string;
		dynamic?: boolean;
	}[];
}

/** `nft -j list ruleset`: a list of single-key objects. */
export interface NftRuleset {
	nftables: Record<string, unknown>[];
}

export interface TotpStatus {
	enabled: boolean;
	pending: boolean;
}

export interface TotpEnrollment {
	secret: string;
	otpauth_url: string;
	qr_code: string;
}

export interface ApiKey {
	id: string;
	name: string;
	scopes: Scope[];
	created_at: number;
	created_by: string;
}

export interface NewApiKey extends ApiKey {
	token: string;
}

export interface ApiKeyList {
	keys: ApiKey[];
	available_scopes: Scope[];
}
