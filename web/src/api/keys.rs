//! API key management (interactive sessions only).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use super::JsonBody;
use crate::{
    auth::{
        SessionUser,
        api_keys::{ApiKeyError, ApiKeyInfo, NewApiKey, Scope},
    },
    error::{ApiError, ApiResult},
    state::AppState,
};

#[derive(Serialize)]
pub struct KeyList {
    keys: Vec<ApiKeyInfo>,
    available_scopes: [Scope; 3],
}

#[derive(Deserialize)]
pub struct CreateKey {
    name: String,
    scopes: Vec<Scope>,
}

pub async fn list(State(state): State<AppState>, _user: SessionUser) -> Json<KeyList> {
    Json(KeyList {
        keys: state.api_keys.list().await,
        available_scopes: Scope::ALL,
    })
}

pub async fn create(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    JsonBody(req): JsonBody<CreateKey>,
) -> ApiResult<(StatusCode, Json<NewApiKey>)> {
    let key = state
        .api_keys
        .create(&req.name, &req.scopes, &user.username)
        .await
        .map_err(|e| match e {
            ApiKeyError::InvalidName | ApiKeyError::NoScopes => ApiError::BadRequest(e.to_string()),
            ApiKeyError::DuplicateName => {
                ApiError::Conflict("an API key with this name already exists")
            }
            ApiKeyError::TooMany => ApiError::Conflict("too many API keys"),
            other => ApiError::internal("creating API key", other),
        })?;
    tracing::info!(user = %user.username, key.id = %key.info.id, key.name = %key.info.name, "API key created");
    Ok((StatusCode::CREATED, Json(key)))
}

pub async fn delete(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let deleted = state
        .api_keys
        .delete(&id)
        .await
        .map_err(|e| ApiError::internal("deleting API key", e))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    tracing::info!(user = %user.username, key.id = %id, "API key deleted");
    Ok(StatusCode::NO_CONTENT)
}
