use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::info;
use utoipa::ToSchema;

use crate::dhcp::AssignLeaseInput;
use crate::routes::AppState;

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct LeasesResponse {
    pub leases: Vec<ApiLease>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiLease {
    pub mac: String,
    pub ip: String,
    pub hostname: Option<String>,
    pub vmid: Option<u32>,
    pub node: Option<String>,
    pub gateway: String,
    pub cidr: u8,
    pub dns_servers: Vec<String>,
    pub lease_start: i64,
    pub lease_end: i64,
    pub state: String,
    pub static_lease: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignLeaseRequest {
    pub vmid: u32,
    pub hostname: String,
    pub ip: String,
    pub gateway: String,
    pub cidr: u8,
    #[serde(default)]
    pub dns_servers: Vec<String>,
    pub lease_time_secs: Option<u64>,
}

#[derive(Serialize, ToSchema)]
pub struct AssignLeaseResponse {
    pub lease: ApiLease,
}

#[derive(Serialize, ToSchema)]
pub struct DeleteLeaseResponse {
    pub removed: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/dhcp/leases",
    tag = "dhcp",
    responses(
        (status = 200, description = "List DHCP leases", body = LeasesResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 503, description = "DHCP service disabled", body = ErrorResponse),
        (status = 500, description = "Failed to load leases", body = ErrorResponse)
    )
)]
pub async fn list_leases(
    State(state): State<AppState>,
) -> Result<Json<LeasesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let dhcp = state
        .dhcp
        .as_ref()
        .ok_or_else(service_disabled_error)?;

    let leases: Vec<ApiLease> = dhcp
        .list_leases()
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(|lease| ApiLease {
            mac: lease.mac,
            ip: lease.ip,
            hostname: lease.hostname,
            vmid: lease.vmid,
            node: lease.node,
            gateway: lease.gateway,
            cidr: lease.cidr,
            dns_servers: lease.dns_servers,
            lease_start: lease.lease_start,
            lease_end: lease.lease_end,
            state: lease.state,
            static_lease: lease.static_lease,
        })
        .collect();

    info!(count = leases.len(), "dhcp leases listed");

    Ok(Json(LeasesResponse { leases }))
}

#[utoipa::path(
    post,
    path = "/api/v1/dhcp/leases",
    tag = "dhcp",
    request_body = AssignLeaseRequest,
    responses(
        (status = 200, description = "Assigned or updated lease", body = AssignLeaseResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 503, description = "DHCP service disabled", body = ErrorResponse),
        (status = 500, description = "Failed to assign lease", body = ErrorResponse)
    )
)]
pub async fn assign_lease(
    State(state): State<AppState>,
    Json(payload): Json<AssignLeaseRequest>,
) -> Result<Json<AssignLeaseResponse>, (StatusCode, Json<ErrorResponse>)> {
    if payload.hostname.trim().is_empty()
        || payload.ip.trim().is_empty()
        || payload.gateway.trim().is_empty()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "vmid, hostname, ip, gateway, and cidr are required".to_owned(),
            }),
        ));
    }

    let vm = state
        .proxmox
        .resolve_qemu_identity(payload.vmid)
        .await
        .map_err(map_identity_error)?;

    info!(
        vmid = vm.vmid,
        node = %vm.node,
        mac = %vm.mac,
        requested_ip = %payload.ip,
        requested_gateway = %payload.gateway,
        requested_cidr = payload.cidr,
        hostname = %payload.hostname,
        "dhcp lease assignment requested"
    );

    let dhcp = state
        .dhcp
        .as_ref()
        .ok_or_else(service_disabled_error)?;

    let lease = dhcp
        .assign_static_lease(AssignLeaseInput {
            mac: vm.mac,
            ip: payload.ip,
            hostname: Some(payload.hostname),
            vmid: Some(vm.vmid),
            node: Some(vm.node),
            gateway: payload.gateway,
            cidr: payload.cidr,
            dns_servers: payload.dns_servers,
            lease_time_secs: payload.lease_time_secs,
        })
        .await
        .map_err(bad_request_or_internal)?;

    info!(
        vmid = lease.vmid,
        node = ?lease.node,
        mac = %lease.mac,
        ip = %lease.ip,
        gateway = %lease.gateway,
        cidr = lease.cidr,
        lease_end = lease.lease_end,
        "dhcp lease assigned"
    );

    Ok(Json(AssignLeaseResponse {
        lease: ApiLease {
            mac: lease.mac,
            ip: lease.ip,
            hostname: lease.hostname,
            vmid: lease.vmid,
            node: lease.node,
            gateway: lease.gateway,
            cidr: lease.cidr,
            dns_servers: lease.dns_servers,
            lease_start: lease.lease_start,
            lease_end: lease.lease_end,
            state: lease.state,
            static_lease: lease.static_lease,
        },
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/dhcp/leases/vm/{vmid}",
    tag = "dhcp",
    params(
        ("vmid" = u32, Path, description = "VMID of the lease to remove")
    ),
    responses(
        (status = 200, description = "Lease removal result", body = DeleteLeaseResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 503, description = "DHCP service disabled", body = ErrorResponse),
        (status = 500, description = "Failed to remove lease", body = ErrorResponse)
    )
)]
pub async fn delete_lease(
    State(state): State<AppState>,
    Path(vmid): Path<u32>,
) -> Result<Json<DeleteLeaseResponse>, (StatusCode, Json<ErrorResponse>)> {
    let dhcp = state
        .dhcp
        .as_ref()
        .ok_or_else(service_disabled_error)?;

    let removed = dhcp
        .remove_lease_by_vmid(vmid)
        .await
        .map_err(internal_error)?;

    if removed {
        info!(vmid, "dhcp lease removed");
    } else {
        info!(vmid, "dhcp lease remove requested but no lease existed");
    }

    Ok(Json(DeleteLeaseResponse { removed }))
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}

fn bad_request_or_internal(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    let message = err.to_string();
    if message.contains("invalid")
        || message.contains("must be")
        || message.contains("not on the same /")
        || message.contains("cannot be combined under")
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: message }),
        );
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}

fn service_disabled_error() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorResponse {
            error: "dhcp service is disabled in config".to_owned(),
        }),
    )
}

fn map_identity_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    let message = err.to_string();
    if message.contains("not found") {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: message }),
        );
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}
