//! Authentication: local Linux accounts, sessions, TOTP and API keys.

pub mod accounts;
pub mod api_keys;
pub mod backend;
pub mod passkeys;
pub mod password;
pub mod principal;
pub mod rate_limit;
pub mod totp;

pub use backend::{Backend, Credentials, User};
pub use principal::{ClientIp, Principal, SessionUser};

pub type AuthSession = axum_login::AuthSession<Backend>;
