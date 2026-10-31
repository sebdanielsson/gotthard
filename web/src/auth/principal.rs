//! Request extractors that establish *who* is calling.
//!
//! - [`Principal`]: a session user or an API key (`Authorization: Bearer`).
//!   Handlers then call [`Principal::require`] with the scope they need.
//! - [`SessionUser`]: an interactive session only. Used for account security
//!   endpoints (TOTP, API key management) so a leaked API key cannot mint new
//!   keys or switch off 2FA.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{header, request::Parts},
};

use super::{
    AuthSession, User,
    api_keys::{ApiKeyInfo, Scope},
    rate_limit::Key,
};
use crate::{error::ApiError, state::AppState};

#[derive(Debug, Clone)]
pub enum Principal {
    Session(User),
    ApiKey(ApiKeyInfo),
}

impl Principal {
    pub fn require(&self, scope: Scope) -> Result<(), ApiError> {
        match self {
            Self::Session(_) => Ok(()),
            Self::ApiKey(key) if key.scopes.contains(&scope) => Ok(()),
            Self::ApiKey(_) => Err(ApiError::Forbidden("API key lacks the required scope")),
        }
    }
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if let Some(token) = bearer_token(parts)? {
            let ip = client_ip(parts);
            let keys = [Key::client(ip)];
            state.limiter.check(&keys).map_err(ApiError::RateLimited)?;
            return match state.api_keys.authenticate(token).await {
                Some(key) => {
                    tracing::debug!(key.id = %key.id, key.name = %key.name, "API key authenticated");
                    Ok(Self::ApiKey(key))
                }
                None => {
                    state.limiter.record_failure(&keys);
                    tracing::warn!(client = %ip, "rejected API token");
                    Err(ApiError::Unauthenticated)
                }
            };
        }
        session_user(parts, state).await.map(Self::Session)
    }
}

/// An authenticated interactive (cookie) session.
#[derive(Debug, Clone)]
pub struct SessionUser(pub User);

impl FromRequestParts<AppState> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if parts.headers.contains_key(header::AUTHORIZATION) {
            return Err(ApiError::Forbidden(
                "this endpoint requires an interactive session",
            ));
        }
        session_user(parts, state).await.map(Self)
    }
}

/// The TCP peer address of the request (there is no reverse proxy in front).
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(client_ip(parts)))
    }
}

async fn session_user(parts: &mut Parts, state: &AppState) -> Result<User, ApiError> {
    let auth = AuthSession::from_request_parts(parts, state)
        .await
        .map_err(|(_, msg)| ApiError::internal("auth session layer", msg))?;
    auth.user.ok_or(ApiError::Unauthenticated)
}

fn client_ip(parts: &Parts) -> IpAddr {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |ci| ci.0.ip())
}

/// Extracts a bearer token. `Ok(None)` if there is no `Authorization` header;
/// an error if there is one but it is not a usable bearer token.
fn bearer_token(parts: &Parts) -> Result<Option<&str>, ApiError> {
    let Some(value) = parts.headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| ApiError::Unauthenticated)?;
    let (scheme, token) = value.split_once(' ').ok_or(ApiError::Unauthenticated)?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.trim().is_empty() {
        return Err(ApiError::Unauthenticated);
    }
    Ok(Some(token.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn parts_with_auth(value: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri("/api/system");
        if let Some(v) = value {
            builder = builder.header(header::AUTHORIZATION, v);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn parses_bearer_tokens() {
        assert_eq!(bearer_token(&parts_with_auth(None)).unwrap(), None);
        assert_eq!(
            bearer_token(&parts_with_auth(Some("Bearer gth_abc"))).unwrap(),
            Some("gth_abc")
        );
        assert_eq!(
            bearer_token(&parts_with_auth(Some("bearer  gth_abc "))).unwrap(),
            Some("gth_abc")
        );
        assert!(bearer_token(&parts_with_auth(Some("Basic YWRtaW46eA=="))).is_err());
        assert!(bearer_token(&parts_with_auth(Some("Bearer"))).is_err());
        assert!(bearer_token(&parts_with_auth(Some("Bearer   "))).is_err());
    }

    #[test]
    fn scope_checks() {
        let key = ApiKeyInfo {
            id: "0011223344556677".into(),
            name: "monitoring".into(),
            scopes: vec![Scope::SystemRead],
            created_at: 0,
            created_by: "admin".into(),
        };
        let principal = Principal::ApiKey(key);
        assert!(principal.require(Scope::SystemRead).is_ok());
        assert!(matches!(
            principal.require(Scope::FirewallRead),
            Err(ApiError::Forbidden(_))
        ));
    }
}
