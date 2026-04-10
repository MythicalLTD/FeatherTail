pub mod api;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Html;
use axum::{Json, Router, routing::get};
use utoipa::OpenApi;

use crate::dhcp::DhcpService;
use crate::proxmox::ProxmoxClient;

#[derive(Clone)]
pub struct AppState {
    pub daemon_name: String,
    pub auth_token: String,
    pub config_path: String,
    pub api_bind: String,
    pub dhcp_enabled: bool,
    pub proxmox: Arc<ProxmoxClient>,
    pub dhcp: Option<Arc<DhcpService>>,
}

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/docs", get(swagger_ui))
        .route("/docs/", get(swagger_ui))
        .route(
            "/openapi.json",
            get(|| async { Json(crate::openapi::ApiDoc::openapi()) }),
        )
        .nest("/api/v1", api::router(state.clone()))
        .fallback(not_found_json)
        .with_state(state)
}

async fn not_found_json() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "404 page not found" })),
    )
}

async fn swagger_ui() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
    <head>
        <meta charset="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>Feathertail API Docs</title>
        <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
        <style>
            body { margin: 0; background: #f7f8fb; }
            .topbar { display: none; }
        </style>
    </head>
    <body>
        <div id="swagger-ui"></div>
        <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
        <script>
            window.ui = SwaggerUIBundle({
                url: '/openapi.json',
                dom_id: '#swagger-ui',
                deepLinking: true,
                displayRequestDuration: true,
                persistAuthorization: true,
            });
        </script>
    </body>
</html>"#,
    )
}
