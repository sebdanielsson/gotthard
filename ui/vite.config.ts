import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Where `npm run dev` proxies /api to: a `gotthard-web --dev` instance.
const apiTarget = process.env.GOTTHARD_API ?? 'http://127.0.0.1:8080';

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Single-page app: gotthard-web embeds build/ and serves index.html for
			// every route that is not a file or under /api.
			adapter: adapter({ fallback: 'index.html' }),
			// Deterministic instead of the default timestamp, so rebuilding the same
			// source yields the same bundle (and the same gotthard-web binary).
			version: { name: process.env.GOTTHARD_VERSION ?? 'dev' },
			// Emitted as a <meta> tag with hashes for SvelteKit's inline bootstrap
			// script. The server adds frame-ancestors etc. as a header.
			csp: {
				mode: 'hash',
				directives: {
					'default-src': ['self'],
					'script-src': ['self'],
					// bits-ui positioning and Svelte transitions use inline styles.
					'style-src': ['self', 'unsafe-inline'],
					'img-src': ['self', 'data:'],
					'font-src': ['self'],
					'connect-src': ['self'],
					'object-src': ['none'],
					'base-uri': ['self'],
					'form-action': ['self']
				}
			}
		})
	],
	server: {
		proxy: {
			'/api': { target: apiTarget }
		}
	}
});
