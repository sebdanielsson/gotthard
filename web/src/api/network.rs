//! `GET /api/interfaces` — iproute2's `ip -j addr`, passed through.

use axum::Json;
use serde::Serialize;

use crate::{
    auth::{Principal, api_keys::Scope},
    command,
    error::{ApiError, ApiResult},
};

#[derive(Serialize)]
pub struct Interfaces {
    /// Unmodified `ip -j addr show` output (an array of links).
    interfaces: serde_json::Value,
}

pub async fn interfaces(principal: Principal) -> ApiResult<Json<Interfaces>> {
    principal.require(Scope::NetworkRead)?;
    let value = command::run_json("ip", &["-j", "addr", "show"]).await?;
    if !value.is_array() {
        return Err(ApiError::ToolFailed {
            tool: "ip",
            message: "expected a JSON array".into(),
        });
    }
    Ok(Json(Interfaces { interfaces: value }))
}
