use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::routes::AppState;

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct ServersResponse {
    pub servers: Vec<ApiServer>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiServer {
    pub vmid: u32,
    pub name: String,
    pub node: String,
    pub status: Option<String>,
    pub mem: Option<u64>,
    pub maxmem: Option<u64>,
    pub dhcp: ApiServerDhcpStatus,
}

#[derive(Serialize, ToSchema)]
pub struct ApiServerDhcpStatus {
    pub enabled: bool,
    pub has_lease: bool,
    pub lease_state: Option<String>,
    pub ip: Option<String>,
    pub lease_end: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/servers",
    tag = "servers",
    responses(
        (status = 200, description = "List of QEMU virtual machines", body = ServersResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn list_servers(
    State(state): State<AppState>,
) -> Result<Json<ServersResponse>, (StatusCode, Json<ErrorResponse>)> {
    let vms = state.proxmox.qemu_list().await.map_err(internal_error)?;

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

    let servers = vms
        .into_iter()
        .map(|vm| ApiServer {
            dhcp: match lease_map.get(&vm.vmid) {
                Some(lease) => ApiServerDhcpStatus {
                    enabled: dhcp_enabled,
                    has_lease: true,
                    lease_state: Some(lease.state.clone()),
                    ip: Some(lease.ip.clone()),
                    lease_end: Some(lease.lease_end),
                },
                None => ApiServerDhcpStatus {
                    enabled: dhcp_enabled,
                    has_lease: false,
                    lease_state: None,
                    ip: None,
                    lease_end: None,
                },
            },
            vmid: vm.vmid,
            name: vm.name,
            node: vm.node,
            status: vm.status,
            mem: vm.mem,
            maxmem: vm.maxmem,
        })
        .collect();

    Ok(Json(ServersResponse { servers }))
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}
