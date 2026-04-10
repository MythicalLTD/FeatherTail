use axum::extract::State;
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
pub struct VersionResponse {
    pub version: ApiProxmoxVersion,
}

#[derive(Serialize, ToSchema)]
pub struct NodesResponse {
    pub nodes: Vec<ApiProxmoxNode>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiProxmoxVersion {
    pub version: String,
    pub release: String,
    pub repoid: String,
}

#[derive(Serialize, ToSchema)]
pub struct ApiProxmoxNode {
    pub node: String,
    pub status: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/proxmox/version",
    tag = "proxmox",
    responses(
        (status = 200, description = "Current Proxmox VE version", body = VersionResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn version(
    State(state): State<AppState>,
) -> Result<Json<VersionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let version = state
        .proxmox
        .version()
        .await
        .map_err(internal_error)?;

    Ok(Json(VersionResponse {
        version: ApiProxmoxVersion {
            version: version.version,
            release: version.release,
            repoid: version.repoid,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/proxmox/nodes",
    tag = "proxmox",
    responses(
        (status = 200, description = "Proxmox cluster nodes", body = NodesResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn nodes(
    State(state): State<AppState>,
) -> Result<Json<NodesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let nodes = state
        .proxmox
        .nodes()
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(|node| ApiProxmoxNode {
            node: node.node,
            status: node.status,
        })
        .collect();

    Ok(Json(NodesResponse { nodes }))
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}
