# gotthard-web

The web UI and JSON API of the Gotthard router OS: a single static Rust binary (axum + tokio) that serves the SvelteKit SPA from [`../ui`](../ui) (embedded at build time with rust-embed) and a JSON API under `/api`.

This is a skeleton: authentication is complete, the system endpoints are read-only.

## Quick start (`--dev` mode)

`--dev` authenticates against fake account files in [`dev/`](dev) instead of `/etc/{passwd,shadow,group}` and keeps its state in `./dev-state`, so it runs without root on macOS or Linux.

| User       | Password   | In `wheel` | Can log in |
| ---------- | ---------- | ---------- | ---------- |
| `admin`    | `gotthard` | yes        | yes        |
| `operator` | `gotthard` | no         | no         |
| `root`     | locked     | yes        | no         |

With a local Rust toolchain (rustup picks up `rust-toolchain.toml`):

```sh
cd web
cargo run -- --dev
```

Without one (for example on the Mac), in the pinned Rust image:

```sh
docker run --rm -it -p 127.0.0.1:8080:8080 \
  -v "$PWD:/src" -v gotthard-web-cache:/cache -w /src/web \
  -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target-dev -e RUSTUP_TOOLCHAIN=1.99.1 \
  rust:1.99.1-alpine3.24@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d \
  cargo run -- --dev --listen 0.0.0.0:8080
```

Run that from the repository root. Then open <http://localhost:8080> (or use `npm run dev` in `ui/` for hot reload; it proxies `/api` to `127.0.0.1:8080`).

Debug builds read the UI from `../ui/build` at runtime, so build the UI once (`npm run build` in `ui/`, or `scripts/build-web.sh`). Without it the API still works and `/` explains what is missing. Release builds embed the UI.

Inside a container `/proc` and `/etc/os-release` describe the container, and `ip`/`nft` are usually missing; the endpoints then return the documented `tool_unavailable` error.

## Building

```sh
scripts/build-web.sh                              # aarch64-unknown-linux-musl (default)
scripts/build-web.sh x86_64-unknown-linux-musl
```

The script builds the UI in `node:26.13.0-alpine3.24` and the backend in `rust:1.99.1-alpine3.24` (both pinned by digest), natively for the target architecture, and writes a static binary to `out/web/gotthard-web-<arch>`. It uses `npm ci --ignore-scripts` and `cargo build --locked`. Caches (npm, cargo registry, target dirs) live in the Docker volume `gotthard-web-cache`; remove it with `docker volume rm gotthard-web-cache`.

Release profile: `opt-level = "s"`, fat LTO, one codegen unit, stripped, because the router runs diskless (from RAM). The aarch64 binary including the UI is about 3 MiB.

## Command-line options

| Flag                     | Default                                   | Description                                                                               |
| ------------------------ | ----------------------------------------- | ----------------------------------------------------------------------------------------- |
| `--listen`               | `127.0.0.1:8080`                          | Listen address (`GOTTHARD_WEB_LISTEN`). TLS is not implemented yet.                       |
| `--state-dir`            | `/etc/gotthard/auth` (dev: `./dev-state`) | TOTP secrets and API keys (`GOTTHARD_WEB_STATE_DIR`).                                     |
| `--dev`                  | off                                       | Use fake account files and relax the production checks.                                   |
| `--passwd-file`          | `dev/passwd`                              | Only with `--dev`.                                                                        |
| `--shadow-file`          | `dev/shadow`                              | Only with `--dev`.                                                                        |
| `--group-file`           | `dev/group`                               | Only with `--dev`.                                                                        |
| `--admin-group`          | `wheel`                                   | Group whose members may log in.                                                           |
| `--session-idle-minutes` | `30`                                      | Session idle timeout (1-1440).                                                            |
| `--insecure-cookies`     | off                                       | Drop `Secure` from the session cookie (plain HTTP on a non-localhost address, or Safari). |

Logging uses `RUST_LOG` (default `info`) and goes to stderr.

## Runtime requirements on the router

- Run as root (reading `/etc/shadow`, `nft list ruleset`). Privilege separation is future work.
- `iproute2` (BusyBox `ip` has no `-j`) and `nftables`.
- The state directory is created `0700`; files in it are written atomically with mode `0600`.
- Diskless Alpine keeps `/etc` in RAM: TOTP enrolments and API keys only survive a reboot after `lbu commit` (with `/etc/gotthard` included in the apkovl). The UI does not trigger that yet.

## Authentication design

### Accounts

Users log in with the router's own Linux accounts. On every login and on every authenticated request the backend re-reads `/etc/passwd`, `/etc/shadow` and `/etc/group` (they are tiny), so password changes, locks, expiry and group removal take effect immediately.

- A user must be a member of `--admin-group` (`wheel`), either as primary group or listed in `/etc/group`.
- Locked (`!`/`*`), empty and expired (shadow field 8) accounts are rejected.
- Supported hashes, all pure Rust so the binary stays static: `$6$` SHA-512-crypt (Alpine default), `$5$` SHA-256-crypt (`sha-crypt`), `$2b$`/`$2y$`/`$2a$` bcrypt (`bcrypt`) and `$y$` yescrypt (RustCrypto `yescrypt`). MD5-crypt and DES are refused.
- Unknown users cost the same as real ones (a dummy SHA-512-crypt verification), so timing does not reveal valid names. Hashing runs on the blocking thread pool. Passwords longer than 1024 bytes are rejected without hashing.

### Sessions

Sessions use tower-sessions (in-memory store; sessions are lost on restart) with axum-login on top.

- Cookie `__Host-gotthard_session`: `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`. The `__Host-` prefix prevents other hosts from planting or overwriting it. Browsers accept `Secure` cookies on `http://localhost` and `http://127.0.0.1`.
- The session id is rotated on login (and when a password-verified login starts waiting for its TOTP code).
- axum-login stores a SHA-256 of the user's shadow hash in the session and compares it on every request: changing the password logs out all sessions.
- Idle timeout 30 minutes by default.
- CSRF: `SameSite=Strict` plus JSON-only request bodies (`Content-Type: application/json` cannot be sent cross-site without a CORS preflight, and no CORS is allowed).

### TOTP (second factor)

RFC 6238 via `totp-rs`: SHA-1, 6 digits, 30-second steps, ±1 step accepted. Secrets are 160 random bits stored base32 in `<state-dir>/totp.json` (`0600`).

1. `POST /api/auth/totp/enroll` creates a *pending* secret and returns it with an `otpauth://` URL and a QR code (SVG data URI).
2. `POST /api/auth/totp/verify` with a valid code activates it.
3. From then on, login is two steps: the password step returns `{"status": "totp_required"}` and parks the user in the (not yet authenticated) session for 5 minutes; `POST /api/auth/login/totp` completes it.

Each accepted time step is recorded, so a code (or an older one in the same window) cannot be replayed. Disabling requires a current code. A user who lost their authenticator can be reset by root by removing their entry from `totp.json`.

### API keys

For automation and MCP servers. Keys are created in the UI (session only), have a name and one or more scopes, and are sent as `Authorization: Bearer <token>`.

- Token format `gth_<16 hex id>_<43 base64url chars>` (256 random bits). The prefix makes leaked tokens easy to find with secret scanners.
- Only an Argon2id hash (default parameters) of the token is stored, in `<state-dir>/api_keys.json` (`0600`). The plaintext is shown once at creation.
- Verification runs on the blocking pool behind a 2-permit semaphore (each Argon2 run needs about 19 MiB). Successful verifications are cached in memory for 5 minutes, keyed by a SHA-256 of the token, and the cache is cleared when a key is deleted.
- Scopes: `system:read`, `network:read`, `firewall:read`. Sessions implicitly have all scopes.
- API keys cannot use the account-security endpoints (`/api/auth/totp*`, `/api/keys*`): a leaked key cannot mint keys or switch off 2FA.

### Rate limiting

In-memory exponential backoff (`src/auth/rate_limit.rs`), tracked separately per client address (IPv6 grouped by /64; password and bearer-token failures both count), per username, and per user for TOTP codes. After 5 consecutive failures a key is blocked for 1 s, doubling per further failure up to 15 minutes; it is forgotten after an hour without failures. Blocked requests get `429` with `Retry-After`. A successful login clears the username key but not the address key.

### Passkeys

Planned. [`src/auth/passkeys.rs`](src/auth/passkeys.rs) documents the intended routes and storage. `webauthn-rs` depends on OpenSSL (vendored for musl), so that choice needs evaluating first; WebAuthn also needs TLS and a stable hostname.

### Logging

Logs include usernames, client addresses, key ids and names, never passwords, TOTP codes, secrets or tokens. Types that hold secrets (`Credentials`, `Account`, `User`, stored key and TOTP records) have redacting `Debug` implementations. A username is only logged if it is syntactically valid, so a password typed into the username field does not end up in the log.

### HTTP hardening

Every response carries `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` and a CSP header with `frame-ancestors 'none'; object-src 'none'; base-uri 'self'; form-action 'self'`. The fetch directives (`default-src 'self'`, `script-src` with the hash of SvelteKit's inline bootstrap script, ...) come from the `<meta>` CSP that SvelteKit writes into `index.html` (see `ui/vite.config.ts`). API responses are `Cache-Control: no-store`; request bodies are limited to 64 KiB.

## API

All endpoints except `POST /api/auth/login` and `POST /api/auth/login/totp` require a session cookie or a bearer token.

| Method   | Path                     | Auth            | Description                                                                              |
| -------- | ------------------------ | --------------- | ---------------------------------------------------------------------------------------- |
| `POST`   | `/api/auth/login`        | none            | `{"username", "password"}` → `{"status": "ok", "user"}` or `{"status": "totp_required"}` |
| `POST`   | `/api/auth/login/totp`   | pending session | `{"code"}` → `{"status": "ok", "user"}`                                                  |
| `POST`   | `/api/auth/logout`       | session         | `204`                                                                                    |
| `GET`    | `/api/auth/me`           | any             | `{"username", "auth_method", "scopes", "totp_enabled" \| "api_key"}`                     |
| `GET`    | `/api/auth/totp`         | session         | `{"enabled", "pending"}`                                                                 |
| `POST`   | `/api/auth/totp/enroll`  | session         | `{"secret", "otpauth_url", "qr_code"}`                                                   |
| `POST`   | `/api/auth/totp/verify`  | session         | `{"code"}` → `{"enabled", "pending"}`                                                    |
| `POST`   | `/api/auth/totp/disable` | session         | `{"code"}` → `{"enabled", "pending"}` (code optional for a pending enrolment)            |
| `GET`    | `/api/keys`              | session         | `{"keys": [...], "available_scopes": [...]}`                                             |
| `POST`   | `/api/keys`              | session         | `{"name", "scopes"}` → `201` with the key and its `token` (once)                         |
| `DELETE` | `/api/keys/{id}`         | session         | `204`, or `404`                                                                          |
| `GET`    | `/api/system`            | `system:read`   | Hostname, kernel release, uptime, load average, memory, os-release                       |
| `GET`    | `/api/interfaces`        | `network:read`  | `{"interfaces": <ip -j addr show>}`                                                      |
| `GET`    | `/api/firewall/ruleset`  | `firewall:read` | `nft -j list ruleset`, unmodified (`{"nftables": [...]}`)                                |

`ip` and `nft` run with a scrubbed environment (`PATH=/usr/sbin:/usr/bin:/sbin:/bin`, `LC_ALL=C`) and a 10-second timeout.

### Errors

Errors are JSON with a stable `code`:

```json
{ "error": { "code": "rate_limited", "message": "too many failed attempts, retry later", "retry_after_seconds": 8 } }
```

| Status | Codes                                                                                        |
| ------ | -------------------------------------------------------------------------------------------- |
| 400    | `bad_request`                                                                                |
| 401    | `unauthenticated`, `invalid_credentials`, `invalid_totp_code`, `no_pending_login`            |
| 403    | `forbidden` (missing scope, or an API key on a session-only endpoint)                        |
| 404    | `not_found`                                                                                  |
| 409    | `conflict`                                                                                   |
| 429    | `rate_limited` (with `Retry-After`)                                                          |
| 500    | `internal` (details are logged, not returned)                                                |
| 502    | `tool_failed` (`ip`/`nft` exited non-zero, timed out or printed invalid JSON)                |
| 503    | `tool_unavailable` (not installed), `tool_permission_denied` (not root / no `CAP_NET_ADMIN`) |

### Example

```sh
# Session
curl -c jar -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"gotthard"}' http://127.0.0.1:8080/api/auth/login
curl -b jar http://127.0.0.1:8080/api/system

# API key (create one under Settings first)
curl -H "Authorization: Bearer $GOTTHARD_TOKEN" http://127.0.0.1:8080/api/interfaces
```

## Code layout

| Path                     | Purpose                                                                            |
| ------------------------ | ---------------------------------------------------------------------------------- |
| `src/main.rs`            | Startup, logging, graceful shutdown                                                |
| `src/config.rs`          | Command-line options                                                               |
| `src/app.rs`             | Router, session layer, security headers (`src/app/tests.rs`: end-to-end API tests) |
| `src/api/`               | Handlers: `auth`, `keys`, `system`, `network`, `firewall`                          |
| `src/auth/accounts.rs`   | passwd/shadow/group parsing and lookups                                            |
| `src/auth/password.rs`   | crypt(3) hash verification                                                         |
| `src/auth/backend.rs`    | axum-login backend (`User`, `Credentials`)                                         |
| `src/auth/principal.rs`  | `Principal` (session or API key) and `SessionUser` extractors                      |
| `src/auth/totp.rs`       | TOTP store, enrolment, replay protection                                           |
| `src/auth/api_keys.rs`   | API key store, Argon2 hashing, scopes                                              |
| `src/auth/rate_limit.rs` | Backoff limiter                                                                    |
| `src/auth/passkeys.rs`   | Passkeys TODO                                                                      |
| `src/secure_fs.rs`       | 0700 dirs, atomic 0600 JSON writes                                                 |
| `src/sysinfo.rs`         | `/proc` and os-release parsing                                                     |
| `src/command.rs`         | Running `ip`/`nft` and parsing their JSON                                          |
| `src/assets.rs`          | Embedded SPA with `index.html` fallback, ETags, caching                            |
| `src/error.rs`           | `ApiError` → JSON responses                                                        |

## Development checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

CI (`.github/workflows/web.yml`) runs these, the UI lint/typecheck/build, and `scripts/build-web.sh` on an `ubuntu-24.04-arm` runner, uploading `gotthard-web-aarch64` as an artifact.
