use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::routes::AppState;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
    pub daemon: String,
}

#[derive(Serialize, ToSchema)]
pub struct StatsResponse {
    pub uptime: Option<u64>,
    pub cpus: Option<u32>,
    pub memory: Option<MemoryStats>,
}

#[derive(Serialize, ToSchema)]
pub struct MemoryStats {
    pub total: u64,
    pub used: u64,
    pub available: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/system/health",
    tag = "system",
    responses(
        (status = 200, description = "Daemon health state", body = HealthResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        daemon: state.daemon_name,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/system/stats",
    tag = "system",
    responses(
        (status = 200, description = "System resource statistics", body = StatsResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse),
        (status = 500, description = "Failed to fetch system stats", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn stats(
    State(_state): State<AppState>,
) -> Result<Json<StatsResponse>, (axum::http::StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    Ok(Json(StatsResponse {
        uptime: None,
        cpus: None,
        memory: None,
    }))
}
