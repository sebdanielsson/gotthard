import { redirect } from '@sveltejs/kit';
import { api } from '#lib/api.js';

export async function load() {
	// Already logged in? Go straight to the dashboard.
	const me = await api.me().catch(() => null);
	if (me) redirect(307, '/');
}
