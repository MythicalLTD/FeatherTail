pub mod containers;
pub mod proxmox;
pub mod servers;
pub mod system;

use axum::middleware;
use axum::{Router, routing::{delete, get, post, put}};

use crate::auth;
use crate::routes::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/system/health", get(system::health))
        .route("/system/stats", get(system::stats))
        .route("/proxmox/version", get(proxmox::version))
        .route("/proxmox/nodes", get(proxmox::nodes))
        .route("/proxmox/execute", post(proxmox::execute))
        .route("/proxmox/json/{*path}", get(proxmox::proxy_json_get))
        .route("/proxmox/json/{*path}", post(proxmox::proxy_json_post))
        .route("/proxmox/json/{*path}", put(proxmox::proxy_json_put))
        .route("/proxmox/json/{*path}", delete(proxmox::proxy_json_delete))
        .route("/proxmox/extjs/{*path}", get(proxmox::proxy_extjs_get))
        .route("/proxmox/extjs/{*path}", post(proxmox::proxy_extjs_post))
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
