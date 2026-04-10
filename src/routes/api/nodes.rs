use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::routes::AppState;

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct NodeDetailsResponse {
    pub uptime: Option<u64>,
    pub cpus: Option<u32>,
    pub memory: Option<ApiMemory>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiMemory {
    pub total: u64,
    pub used: u64,
    pub available: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/nodes/{node}",
    tag = "nodes",
    responses(
        (status = 200, description = "Node status and information", body = NodeDetailsResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn node_details(
    State(state): State<AppState>,
    Path(node): Path<String>,
) -> Result<Json<NodeDetailsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let status = state
        .proxmox
        .node_status(&node)
        .await
        .map_err(internal_error)?;

    let memory = status.memory.map(|m| ApiMemory {
        total: m.total,
        used: m.used,
        available: m.available,
    });

    let cpus = status.cpuinfo.map(|c| c.cpus);

    Ok(Json(NodeDetailsResponse {
        uptime: status.uptime,
        cpus,
        memory,
    }))
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}
