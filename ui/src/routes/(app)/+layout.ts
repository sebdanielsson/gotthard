import { redirect } from '@sveltejs/kit';
import { api, isUnauthenticated } from '#lib/api.js';

export async function load() {
	try {
		return { me: await api.me() };
	} catch (e) {
		if (isUnauthenticated(e)) redirect(307, '/login');
		throw e;
	}
}
