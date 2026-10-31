//! `GET /api/system`

use axum::Json;

use crate::{
    auth::{Principal, api_keys::Scope},
    error::ApiResult,
    sysinfo::{self, SystemInfo},
};

pub async fn get(principal: Principal) -> ApiResult<Json<SystemInfo>> {
    principal.require(Scope::SystemRead)?;
    Ok(Json(sysinfo::collect().await))
}
