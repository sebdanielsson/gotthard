//! JSON API mounted under `/api`.

mod auth;
mod firewall;
mod keys;
mod network;
mod system;

use axum::{
    Json, Router,
    extract::{FromRequest, Request, rejection::JsonRejection},
    routing::{delete, get, post},
};
use serde::de::DeserializeOwned;

use crate::{error::ApiError, state::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/login/totp", post(auth::login_totp))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/totp", get(auth::totp_status))
        .route("/auth/totp/enroll", post(auth::totp_enroll))
        .route("/auth/totp/verify", post(auth::totp_verify))
        .route("/auth/totp/disable", post(auth::totp_disable))
        .route("/keys", get(keys::list).post(keys::create))
        .route("/keys/{id}", delete(keys::delete))
        .route("/system", get(system::get))
        .route("/interfaces", get(network::interfaces))
        .route("/firewall/ruleset", get(firewall::ruleset))
        .fallback(|| async { ApiError::NotFound })
}

/// `axum::Json` with rejections rendered as structured [`ApiError`]s.
/// Requiring `Content-Type: application/json` also means a cross-site form
/// cannot hit these endpoints without a CORS preflight.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|rejection: JsonRejection| ApiError::BadRequest(rejection.body_text()))
    }
}
