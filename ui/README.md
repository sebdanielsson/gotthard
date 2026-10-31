# Gotthard web UI

SvelteKit (Svelte 5) single-page app with Tailwind CSS v4 and shadcn-svelte components. It is built with `adapter-static` and embedded into the `gotthard-web` binary; see [`../web/README.md`](../web/README.md) for the backend, the API and the auth design.

## Development

Node and npm versions are pinned in `.node-version` and `package.json` (`packageManager`, `devEngines`); npm refuses to run with other versions.

Start the backend in dev mode (see `../web/README.md`), then:

```sh
npm ci
npm run dev        # http://localhost:5173, proxies /api to 127.0.0.1:8080 (override with GOTTHARD_API)
```

Log in as `admin` / `gotthard` (from `../web/dev/shadow`).

## Checks and build

```sh
npm run lint       # prettier + eslint
npm run check      # svelte-check (TypeScript)
npm run build      # writes build/, which gotthard-web embeds
```

`../scripts/build-web.sh` runs the build inside the pinned `node` container image.

## Components

UI primitives in `src/lib/components/ui/` come from shadcn-svelte (`components.json`, style "vega"). Add more with:

```sh
npx shadcn-svelte@1.7.2 add <component>
```
