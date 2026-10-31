//! Passkeys (WebAuthn) — planned, not implemented yet.
//!
//! TODO(passkeys): implement registration and authentication ceremonies.
//!
//! Intended design:
//! - Routes (session-only, like TOTP):
//!   - `POST /api/auth/passkeys/register/start`  -> `CreationChallengeResponse`
//!   - `POST /api/auth/passkeys/register/finish` <- `RegisterPublicKeyCredential`
//!   - `POST /api/auth/passkeys/login/start`     -> `RequestChallengeResponse`
//!   - `POST /api/auth/passkeys/login/finish`    <- `PublicKeyCredential`
//!   - `GET/DELETE /api/auth/passkeys[/{id}]` to list and remove credentials.
//! - Ceremony state (`PasskeyRegistration` / `PasskeyAuthentication`) is kept in
//!   the tower-sessions session between `start` and `finish`.
//! - Credentials are stored per user in `<state_dir>/passkeys.json` (0600)
//!   through [`crate::secure_fs`], like TOTP secrets and API keys.
//! - A passkey login must still pass the admin-group check in
//!   [`super::backend::Backend`] (`get_user`), so removing a user from `wheel`
//!   revokes passkey access too.
//! - The relying-party ID must be a stable hostname, which is why this waits
//!   for TLS and a configured hostname (WebAuthn requires a secure context).
//!
//! Dependency note: `webauthn-rs` (0.6.x) depends on OpenSSL. For the static
//! musl build that means `openssl` with the `vendored` feature (adds a C build
//! of OpenSSL and roughly 1-2 MB to the binary). Evaluate a pure-Rust
//! alternative before committing to it.
