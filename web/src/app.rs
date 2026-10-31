//! Router assembly: API, sessions, static UI and cross-cutting layers.

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, header},
};
use axum_login::AuthManagerLayerBuilder;
use tower_http::{
    set_header::SetResponseHeaderLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer, cookie::SameSite};
use tracing::Level;

use crate::{api, assets, auth::Backend, config::Config, state::AppState};

const API_BODY_LIMIT: usize = 64 * 1024;

/// Applied to every response. The fetch directives (`default-src`,
/// `script-src` with the hash of SvelteKit's inline bootstrap script, ...) live
/// in the `<meta>` CSP that SvelteKit writes into `index.html` (configured in
/// `ui/vite.config.ts`). Browsers enforce the intersection of both policies,
/// so repeating `script-src 'self'` here would block that hashed script. This
/// header only adds what a `<meta>` policy cannot express.
const CSP: &str = "frame-ancestors 'none'; object-src 'none'; base-uri 'self'; form-action 'self'";

pub fn build(state: AppState, backend: Backend, config: &Config) -> Router {
    let cookie_name = if config.secure_cookies {
        // `__Host-` binds the cookie to this exact origin (Secure, Path=/,
        // no Domain), so a sibling host cannot plant or overwrite it.
        "__Host-gotthard_session"
    } else {
        "gotthard_session"
    };
    let idle = i64::try_from(config.session_idle.as_secs()).unwrap_or(30 * 60);
    let sessions = SessionManagerLayer::new(MemoryStore::default())
        .with_name(cookie_name)
        .with_secure(config.secure_cookies)
        .with_http_only(true)
        .with_same_site(SameSite::Strict)
        .with_path("/")
        .with_expiry(Expiry::OnInactivity(
            tower_sessions::cookie::time::Duration::seconds(idle),
        ));
    let auth = AuthManagerLayerBuilder::new(backend, sessions).build();

    let api = api::router()
        .layer(auth)
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));

    Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .layer(header_layer(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(header_layer(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(header_layer(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(header_layer(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state)
}

fn header_layer(name: HeaderName, value: HeaderValue) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, value)
}

#[cfg(test)]
mod tests;
