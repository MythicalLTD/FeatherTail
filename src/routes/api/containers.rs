use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::routes::AppState;

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct ContainersResponse {
    pub containers: Vec<ApiContainer>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiContainer {
    pub vmid: u32,
    pub name: String,
    pub node: String,
    pub status: Option<String>,
    pub mem: Option<u64>,
    pub maxmem: Option<u64>,
    pub dhcp: ApiContainerDhcpStatus,
}

#[derive(Serialize, ToSchema)]
pub struct ApiContainerDhcpStatus {
    pub enabled: bool,
    pub has_lease: bool,
    pub lease_state: Option<String>,
    pub ip: Option<String>,
    pub lease_end: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetRootPasswordRequest {
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct SetRootPasswordResponse {
    pub vmid: u32,
    pub node: String,
    pub message: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/containers",
    tag = "containers",
    responses(
        (status = 200, description = "List of LXC containers", body = ContainersResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn list_containers(
    State(state): State<AppState>,
) -> Result<Json<ContainersResponse>, (StatusCode, Json<ErrorResponse>)> {
    let containers = state
        .proxmox
        .lxc_list()
        .await
        .map_err(internal_error)?;

    let lease_map = if let Some(dhcp) = state.dhcp.as_ref() {
        if dhcp.enabled() {
            let leases = dhcp.list_leases().await.map_err(internal_error)?;
            let mut map = HashMap::new();
            for lease in leases {
                if let Some(vmid) = lease.vmid {
                    map.insert(vmid, lease);
                }
            }
            map
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    let dhcp_enabled = state
        .dhcp
        .as_ref()
        .map(|dhcp| dhcp.enabled())
        .unwrap_or(false);

    let containers = containers
        .into_iter()
        .map(|ct| ApiContainer {
            dhcp: match lease_map.get(&ct.vmid) {
                Some(lease) => ApiContainerDhcpStatus {
                    enabled: dhcp_enabled,
                    has_lease: true,
                    lease_state: Some(lease.state.clone()),
                    ip: Some(lease.ip.clone()),
                    lease_end: Some(lease.lease_end),
                },
                None => ApiContainerDhcpStatus {
                    enabled: dhcp_enabled,
                    has_lease: false,
                    lease_state: None,
                    ip: None,
                    lease_end: None,
                },
            },
            vmid: ct.vmid,
            name: ct.name,
            node: ct.node,
            status: ct.status,
            mem: ct.mem,
            maxmem: ct.maxmem,
        })
        .collect();

    Ok(Json(ContainersResponse { containers }))
}

#[utoipa::path(
    post,
    path = "/api/v1/containers/{vmid}/root-password",
    tag = "containers",
    request_body = SetRootPasswordRequest,
    params(
        ("vmid" = u32, Path, description = "LXC VMID")
    ),
    responses(
        (status = 200, description = "Root password updated", body = SetRootPasswordResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Container not found", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn set_root_password(
    State(state): State<AppState>,
    Path(vmid): Path<u32>,
    Json(payload): Json<SetRootPasswordRequest>,
) -> Result<Json<SetRootPasswordResponse>, (StatusCode, Json<ErrorResponse>)> {
    if payload.password.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "password cannot be empty".to_string(),
            }),
        ));
    }

    let node = state
        .proxmox
        .set_lxc_root_password(vmid, &payload.password)
        .await
        .map_err(map_password_change_error)?;

    Ok(Json(SetRootPasswordResponse {
        vmid,
        node,
        message: "root password updated".to_string(),
    }))
}

fn map_password_change_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    let error = err.to_string();
    if error.contains("not found") {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error }),
        );
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error }),
    )
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}
