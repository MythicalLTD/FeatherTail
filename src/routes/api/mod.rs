pub mod containers;
pub mod proxmox;
pub mod servers;
pub mod system;

use axum::middleware;
use axum::{Router, routing::{get, post}};

use crate::auth;
use crate::routes::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/system/health", get(system::health))
        .route("/system/stats", get(system::stats))
        .route("/proxmox/version", get(proxmox::version))
        .route("/proxmox/nodes", get(proxmox::nodes))
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
