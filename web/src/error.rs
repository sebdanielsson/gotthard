//! Structured API errors.
//!
//! Every failure that reaches a client is rendered as
//! `{"error": {"code": "...", "message": "..."}}` with a matching HTTP status.
//! Internal details are logged, never returned.

use std::time::Duration;

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("authentication required")]
    Unauthenticated,

    #[error("invalid username or password")]
    InvalidCredentials,

    #[error("invalid or expired verification code")]
    InvalidTotpCode,

    #[error("no login is awaiting a verification code")]
    NoPendingLogin,

    #[error("{0}")]
    Forbidden(&'static str),

    #[error("too many failed attempts, retry later")]
    RateLimited(Duration),

    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Conflict(&'static str),

    #[error("not found")]
    NotFound,

    #[error("`{tool}` is not installed on this system")]
    ToolUnavailable { tool: &'static str },

    #[error("insufficient privileges to run `{tool}`")]
    ToolPermissionDenied { tool: &'static str },

    #[error("`{tool}` failed: {message}")]
    ToolFailed { tool: &'static str, message: String },

    /// Anything unexpected. The detail is logged but not sent to the client.
    #[error("internal server error")]
    Internal(String),
}

impl ApiError {
    pub fn internal(context: &str, err: impl std::fmt::Display) -> Self {
        Self::Internal(format!("{context}: {err}"))
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated
            | Self::InvalidCredentials
            | Self::InvalidTotpCode
            | Self::NoPendingLogin => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ToolUnavailable { .. } | Self::ToolPermissionDenied { .. } => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::ToolFailed { .. } => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::InvalidCredentials => "invalid_credentials",
            Self::InvalidTotpCode => "invalid_totp_code",
            Self::NoPendingLogin => "no_pending_login",
            Self::Forbidden(_) => "forbidden",
            Self::RateLimited(_) => "rate_limited",
            Self::BadRequest(_) => "bad_request",
            Self::Conflict(_) => "conflict",
            Self::NotFound => "not_found",
            Self::ToolUnavailable { .. } => "tool_unavailable",
            Self::ToolPermissionDenied { .. } => "tool_permission_denied",
            Self::ToolFailed { .. } => "tool_failed",
            Self::Internal(_) => "internal",
        }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(detail) = &self {
            tracing::error!(%detail, "request failed with internal error");
        }

        let retry_after = match &self {
            // Round up so clients never retry a fraction of a second too early.
            Self::RateLimited(wait) => Some(wait.as_secs() + u64::from(wait.subsec_nanos() > 0)),
            _ => None,
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code(),
                message: self.to_string(),
                retry_after_seconds: retry_after,
            },
        };

        let mut response = (self.status(), Json(body)).into_response();
        if let Some(secs) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn rate_limited_sets_retry_after() {
        let response = ApiError::RateLimited(Duration::from_millis(1500)).into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "rate_limited");
        assert_eq!(json["error"]["retry_after_seconds"], 2);
    }

    #[tokio::test]
    async fn internal_errors_hide_details() {
        let response = ApiError::internal("reading secret", "disk on fire").into_response();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("disk on fire"));
        assert!(text.contains("internal server error"));
    }
}
