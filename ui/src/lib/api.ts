import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import type {
	ApiKeyList,
	IpLink,
	LoginResponse,
	Me,
	NewApiKey,
	NftRuleset,
	Scope,
	SystemInfo,
	TotpEnrollment,
	TotpStatus
} from './types';

/** An error response from the API (`{"error": {"code", "message"}}`). */
export class ApiError extends Error {
	constructor(
		readonly status: number,
		readonly code: string,
		message: string,
		readonly retryAfter?: number
	) {
		super(message);
		this.name = 'ApiError';
	}
}

interface ErrorBody {
	error?: { code?: string; message?: string; retry_after_seconds?: number };
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
	const headers: Record<string, string> = { Accept: 'application/json' };
	if (body !== undefined) headers['Content-Type'] = 'application/json';

	let res: Response;
	try {
		res = await fetch(`/api${path}`, {
			method,
			headers,
			body: body === undefined ? undefined : JSON.stringify(body),
			credentials: 'same-origin'
		});
	} catch {
		throw new ApiError(0, 'network', 'Cannot reach the router.');
	}

	if (res.status === 204) return undefined as T;
	const data: unknown = await res.json().catch(() => null);
	if (!res.ok) {
		const err = (data as ErrorBody | null)?.error;
		throw new ApiError(
			res.status,
			err?.code ?? 'unknown',
			err?.message ?? `${res.status} ${res.statusText}`,
			err?.retry_after_seconds
		);
	}
	return data as T;
}

export const api = {
	me: () => request<Me>('GET', '/auth/me'),
	login: (username: string, password: string) =>
		request<LoginResponse>('POST', '/auth/login', { username, password }),
	loginTotp: (code: string) => request<LoginResponse>('POST', '/auth/login/totp', { code }),
	logout: () => request<void>('POST', '/auth/logout'),

	system: () => request<SystemInfo>('GET', '/system'),
	interfaces: () => request<{ interfaces: IpLink[] }>('GET', '/interfaces'),
	ruleset: () => request<NftRuleset>('GET', '/firewall/ruleset'),

	totpStatus: () => request<TotpStatus>('GET', '/auth/totp'),
	totpEnroll: () => request<TotpEnrollment>('POST', '/auth/totp/enroll'),
	totpVerify: (code: string) => request<TotpStatus>('POST', '/auth/totp/verify', { code }),
	totpDisable: (code: string) => request<TotpStatus>('POST', '/auth/totp/disable', { code }),

	listKeys: () => request<ApiKeyList>('GET', '/keys'),
	createKey: (name: string, scopes: Scope[]) =>
		request<NewApiKey>('POST', '/keys', { name, scopes }),
	deleteKey: (id: string) => request<void>('DELETE', `/keys/${encodeURIComponent(id)}`)
};

export function isUnauthenticated(e: unknown): boolean {
	return e instanceof ApiError && e.code === 'unauthenticated';
}

/**
 * Human-readable message for an error. If the session has expired, navigates
 * to the login page as a side effect.
 */
export function errorMessage(e: unknown): string {
	if (isUnauthenticated(e)) {
		void goto(resolve('/login'));
		return 'Your session has expired.';
	}
	if (e instanceof ApiError) {
		if (e.code === 'rate_limited' && e.retryAfter) {
			return `Too many attempts. Try again in ${e.retryAfter} s.`;
		}
		return e.message;
	}
	return e instanceof Error ? e.message : String(e);
}
