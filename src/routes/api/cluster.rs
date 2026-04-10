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
pub struct ClusterInfoResponse {
    pub name: String,
    pub nodeid: u32,
}

#[derive(Serialize, ToSchema)]
pub struct ClusterNodesResponse {
    pub nodes: Vec<ApiClusterNode>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiClusterNode {
    pub nodeid: u32,
    pub name: String,
    pub online: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/api/v1/cluster/info",
    tag = "cluster",
    responses(
        (status = 200, description = "Cluster information", body = ClusterInfoResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn cluster_info(
    State(state): State<AppState>,
) -> Result<Json<ClusterInfoResponse>, (StatusCode, Json<ErrorResponse>)> {
    let status = state
        .proxmox
        .cluster_status()
        .await
        .map_err(internal_error)?;

    let (name, nodeid) = status
        .cluster
        .map(|c| (c.name, c.nodeid))
        .ok_or_else(|| {
            internal_error("cluster information not available")
        })?;

    Ok(Json(ClusterInfoResponse { name, nodeid }))
}

#[utoipa::path(
    get,
    path = "/api/v1/cluster/nodes",
    tag = "cluster",
    responses(
        (status = 200, description = "Cluster nodes", body = ClusterNodesResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn cluster_nodes(
    State(state): State<AppState>,
) -> Result<Json<ClusterNodesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let status = state
        .proxmox
        .cluster_status()
        .await
        .map_err(internal_error)?;

    let nodes = status
        .nodelist
        .unwrap_or_default()
        .into_iter()
        .map(|node| ApiClusterNode {
            nodeid: node.nodeid,
            name: node.name,
            online: node.online,
        })
        .collect();

    Ok(Json(ClusterNodesResponse { nodes }))
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}
