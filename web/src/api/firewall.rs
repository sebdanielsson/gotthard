//! `GET /api/firewall/ruleset` — `nft -j list ruleset`, passed through.

use axum::Json;

use crate::{
    auth::{Principal, api_keys::Scope},
    command,
    error::{ApiError, ApiResult},
};

pub async fn ruleset(principal: Principal) -> ApiResult<Json<serde_json::Value>> {
    principal.require(Scope::FirewallRead)?;
    let value = command::run_json("nft", &["-j", "list", "ruleset"]).await?;
    if !value
        .get("nftables")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err(ApiError::ToolFailed {
            tool: "nft",
            message: "expected an object with an `nftables` array".into(),
        });
    }
    Ok(Json(value))
}
