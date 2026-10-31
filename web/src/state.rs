//! Shared application state handed to every handler.

use std::sync::Arc;

use crate::auth::{api_keys::ApiKeyStore, rate_limit::RateLimiter, totp::TotpStore};

#[derive(Debug, Clone)]
pub struct AppState {
    pub totp: Arc<TotpStore>,
    pub api_keys: Arc<ApiKeyStore>,
    pub limiter: Arc<RateLimiter>,
    /// Used to label TOTP entries in authenticator apps (`user@hostname`).
    pub hostname: Arc<str>,
}
