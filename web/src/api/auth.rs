//! Login, logout, identity and TOTP management.
//!
//! Login is one or two steps:
//! 1. `POST /api/auth/login` with username and password. Without TOTP this
//!    logs in directly. With TOTP the password result is parked in the
//!    (not yet authenticated) session and `{"status": "totp_required"}` is
//!    returned.
//! 2. `POST /api/auth/login/totp` with the code completes the login.

use axum::{Json, extract::State, http::StatusCode};
use axum_login::AuthnBackend as _;
use serde::{Deserialize, Serialize};

use super::JsonBody;
use crate::{
    auth::{
        AuthSession, ClientIp, Credentials, Principal, SessionUser, User,
        accounts::is_valid_username,
        api_keys::Scope,
        password::MAX_PASSWORD_LEN,
        rate_limit::Key,
        totp::{Enrollment, TotpError, TotpStatus},
    },
    error::{ApiError, ApiResult},
    state::AppState,
};

const PENDING_KEY: &str = "gotthard.pending_totp";
/// How long a password-verified login may wait for its TOTP code.
const PENDING_TTL_SECS: u64 = 300;

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct CodeRequest {
    code: String,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginResponse {
    Ok { user: Me },
    TotpRequired,
}

#[derive(Serialize, Deserialize)]
struct PendingLogin {
    username: String,
    issued_at: u64,
}

#[derive(Serialize)]
pub struct Me {
    username: String,
    /// `session` or `api_key`.
    auth_method: &'static str,
    scopes: Vec<Scope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    totp_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<ApiKeyRef>,
}

#[derive(Serialize)]
struct ApiKeyRef {
    id: String,
    name: String,
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    mut auth: AuthSession,
    JsonBody(req): JsonBody<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    if req.username.is_empty() || req.username.len() > 64 || req.password.len() > MAX_PASSWORD_LEN {
        return Err(ApiError::BadRequest(
            "invalid username or password length".into(),
        ));
    }
    // Only well-formed names are logged, so a password typed into the
    // username field never ends up in the logs.
    let log_name = if is_valid_username(&req.username) {
        req.username.as_str()
    } else {
        "<invalid>"
    };

    let keys = [Key::client(ip), Key::User(req.username.clone())];
    state.limiter.check(&keys).map_err(|wait| {
        tracing::warn!(user = %log_name, client = %ip, "login rate limited");
        ApiError::RateLimited(wait)
    })?;

    let user = auth
        .authenticate(Credentials {
            username: req.username.clone(),
            password: req.password,
        })
        .await
        .map_err(|e| ApiError::internal("authenticating", e))?;
    let Some(user) = user else {
        state.limiter.record_failure(&keys);
        tracing::warn!(user = %log_name, client = %ip, "login failed");
        return Err(ApiError::InvalidCredentials);
    };

    if state.totp.is_enabled(&user.username).await {
        // Fresh id for the half-authenticated session, too.
        auth.session
            .cycle_id()
            .await
            .map_err(|e| ApiError::internal("cycling session id", e))?;
        auth.session
            .insert(
                PENDING_KEY,
                PendingLogin {
                    username: user.username.clone(),
                    issued_at: unix_now(),
                },
            )
            .await
            .map_err(|e| ApiError::internal("storing pending login", e))?;
        tracing::info!(user = %user.username, client = %ip, "password accepted, awaiting TOTP code");
        return Ok(Json(LoginResponse::TotpRequired));
    }

    complete_login(&state, &mut auth, &user, ip).await
}

pub async fn login_totp(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    mut auth: AuthSession,
    JsonBody(req): JsonBody<CodeRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let pending: Option<PendingLogin> = auth
        .session
        .get(PENDING_KEY)
        .await
        .map_err(|e| ApiError::internal("reading pending login", e))?;
    let pending = pending
        .filter(|p| unix_now().saturating_sub(p.issued_at) <= PENDING_TTL_SECS)
        .ok_or(ApiError::NoPendingLogin)?;

    let keys = [Key::client(ip), Key::Totp(pending.username.clone())];
    state.limiter.check(&keys).map_err(|wait| {
        tracing::warn!(user = %pending.username, client = %ip, "TOTP rate limited");
        ApiError::RateLimited(wait)
    })?;

    match state.totp.verify(&pending.username, &req.code).await {
        Ok(()) => {}
        Err(TotpError::InvalidCode | TotpError::NotEnrolled) => {
            state.limiter.record_failure(&keys);
            tracing::warn!(user = %pending.username, client = %ip, "invalid TOTP code at login");
            return Err(ApiError::InvalidTotpCode);
        }
        Err(e) => return Err(ApiError::internal("verifying TOTP", e)),
    }

    // Re-check the account: it may have been locked or removed from the
    // admin group since the password step.
    let user = auth
        .backend
        .get_user(&pending.username)
        .await
        .map_err(|e| ApiError::internal("loading user", e))?
        .ok_or(ApiError::InvalidCredentials)?;
    auth.session
        .remove::<PendingLogin>(PENDING_KEY)
        .await
        .map_err(|e| ApiError::internal("clearing pending login", e))?;
    state
        .limiter
        .record_success(&[Key::Totp(user.username.clone())]);
    complete_login(&state, &mut auth, &user, ip).await
}

async fn complete_login(
    state: &AppState,
    auth: &mut AuthSession,
    user: &User,
    ip: std::net::IpAddr,
) -> ApiResult<Json<LoginResponse>> {
    auth.login(user)
        .await
        .map_err(|e| ApiError::internal("establishing session", e))?;
    state
        .limiter
        .record_success(&[Key::User(user.username.clone())]);
    tracing::info!(user = %user.username, client = %ip, "login succeeded");
    Ok(Json(LoginResponse::Ok {
        user: session_me(state, user).await,
    }))
}

pub async fn logout(mut auth: AuthSession) -> ApiResult<StatusCode> {
    let user = auth
        .logout()
        .await
        .map_err(|e| ApiError::internal("ending session", e))?;
    if let Some(user) = user {
        tracing::info!(user = %user.username, "logged out");
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(State(state): State<AppState>, principal: Principal) -> Json<Me> {
    Json(match principal {
        Principal::Session(user) => session_me(&state, &user).await,
        Principal::ApiKey(key) => Me {
            username: key.created_by,
            auth_method: "api_key",
            scopes: key.scopes,
            totp_enabled: None,
            api_key: Some(ApiKeyRef {
                id: key.id,
                name: key.name,
            }),
        },
    })
}

async fn session_me(state: &AppState, user: &User) -> Me {
    Me {
        username: user.username.clone(),
        auth_method: "session",
        scopes: Scope::ALL.to_vec(),
        totp_enabled: Some(state.totp.is_enabled(&user.username).await),
        api_key: None,
    }
}

pub async fn totp_status(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Json<TotpStatus> {
    Json(state.totp.status(&user.username).await)
}

pub async fn totp_enroll(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> ApiResult<Json<Enrollment>> {
    let label = format!("{}@{}", user.username, state.hostname);
    let enrollment = state
        .totp
        .begin_enrollment(&user.username, &label)
        .await
        .map_err(totp_error)?;
    tracing::info!(user = %user.username, "TOTP enrolment started");
    Ok(Json(enrollment))
}

pub async fn totp_verify(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    SessionUser(user): SessionUser,
    JsonBody(req): JsonBody<CodeRequest>,
) -> ApiResult<Json<TotpStatus>> {
    let keys = [Key::Totp(user.username.clone())];
    state.limiter.check(&keys).map_err(ApiError::RateLimited)?;
    if let Err(e) = state
        .totp
        .confirm_enrollment(&user.username, &req.code)
        .await
    {
        if matches!(e, TotpError::InvalidCode) {
            state.limiter.record_failure(&keys);
        }
        return Err(totp_error(e));
    }
    state.limiter.record_success(&keys);
    tracing::info!(user = %user.username, client = %ip, "TOTP enabled");
    Ok(Json(state.totp.status(&user.username).await))
}

pub async fn totp_disable(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    SessionUser(user): SessionUser,
    JsonBody(req): JsonBody<CodeRequest>,
) -> ApiResult<Json<TotpStatus>> {
    let keys = [Key::Totp(user.username.clone())];
    state.limiter.check(&keys).map_err(ApiError::RateLimited)?;
    if let Err(e) = state.totp.disable(&user.username, &req.code).await {
        if matches!(e, TotpError::InvalidCode) {
            state.limiter.record_failure(&keys);
        }
        return Err(totp_error(e));
    }
    state.limiter.record_success(&keys);
    tracing::info!(user = %user.username, client = %ip, "TOTP disabled");
    Ok(Json(state.totp.status(&user.username).await))
}

fn totp_error(e: TotpError) -> ApiError {
    match e {
        TotpError::InvalidCode => ApiError::InvalidTotpCode,
        TotpError::AlreadyEnabled => {
            ApiError::Conflict("two-factor authentication is already enabled")
        }
        TotpError::NotEnrolled => ApiError::Conflict("two-factor authentication is not set up"),
        other => ApiError::internal("TOTP", other),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
