pub mod admin;
pub mod containers;
pub mod dhcp;
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
        // Admin web UI endpoints (session cookie or bearer token).
        .route("/admin/overview", get(admin::overview))
        .route("/admin/bridges", get(admin::bridges))
        .route("/admin/guests", get(admin::guests))
        .route("/admin/pool", get(admin::list_pool).post(admin::add_pool))
        .route("/admin/pool/{ip}", put(admin::update_pool).delete(admin::delete_pool))
        .route("/admin/leases", get(admin::list_leases).post(admin::create_lease))
        .route("/admin/leases/{mac}", delete(admin::delete_lease))
        .route("/admin/leases/{mac}/renew", post(admin::renew_lease))
        .route("/admin/leases/{mac}/check", post(admin::check_lease))
        .route("/admin/settings", get(admin::get_settings).put(admin::update_settings))
        .route("/admin/restart", post(admin::restart))
        .route("/auth/password", post(auth::change_password))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ))
        // Public: login/session handshake for the web UI.
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/session", get(auth::session))
}
