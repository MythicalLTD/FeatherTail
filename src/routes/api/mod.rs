pub mod containers;
pub mod dhcp;
pub mod proxmox;
pub mod servers;
pub mod system;

use axum::middleware;
use axum::{Router, routing::{get, post, put}};

use crate::auth;
use crate::routes::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/system/health", get(system::health))
        .route("/system/stats", get(system::stats))
        .route("/system/diagnostics", get(system::diagnostics))
        .route("/system/logs", get(system::logs))
        .route("/system/config", get(system::get_config))
        .route("/system/config", put(system::update_config))
        .route("/system/restart", post(system::restart))
        .route("/system/update", post(system::self_update))
        .route("/proxmox/version", get(proxmox::version))
        .route("/proxmox/nodes", get(proxmox::nodes))
        .route("/dhcp/leases", get(dhcp::list_leases))
        .route("/dhcp/leases", post(dhcp::assign_lease))
        .route("/dhcp/leases/vm/{vmid}", axum::routing::delete(dhcp::delete_lease))
        .route("/servers", get(servers::list_servers))
        .route("/containers", get(containers::list_containers))
        .route(
            "/containers/{vmid}/root-password",
            post(containers::set_root_password),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer_auth,
        ))
}
