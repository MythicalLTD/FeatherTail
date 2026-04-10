use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::routes::AppState;

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ExecuteProxmoxRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
pub struct ExecuteProxmoxResponse {
    pub result: Value,
}

#[derive(Deserialize, ToSchema)]
pub struct ProxyParams {
    #[serde(flatten)]
    pub params: BTreeMap<String, String>,
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

#[utoipa::path(
    post,
    path = "/api/v1/proxmox/execute",
    tag = "proxmox",
    request_body = ExecuteProxmoxRequest,
    responses(
        (status = 200, description = "Raw Proxmox pvesh command result", body = ExecuteProxmoxResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn execute(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteProxmoxRequest>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    let method = payload.method.trim().to_lowercase();
    if !matches!(method.as_str(), "get" | "set" | "create" | "delete") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "method must be one of: get, set, create, delete".to_string(),
            }),
        ));
    }

    let path = payload.path.trim();
    if !path.starts_with('/') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "path must start with '/'".to_string(),
            }),
        ));
    }

    let params = payload.params.into_iter().collect::<Vec<(String, String)>>();
    let result = state
        .proxmox
        .execute(&method, path, &params)
        .await
        .map_err(internal_error)?;

    Ok(Json(ExecuteProxmoxResponse { result }))
}

#[utoipa::path(
    get,
    path = "/api/v1/proxmox/json/{path}",
    tag = "proxmox",
    params(
        ("path" = String, Path, description = "Relative Proxmox JSON path without /api2/json prefix")
    ),
    responses(
        (status = 200, description = "Raw result", body = ExecuteProxmoxResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn proxy_json_get(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ProxyParams>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    proxy_call(state, "get", path, query.params).await
}

#[utoipa::path(
    post,
    path = "/api/v1/proxmox/json/{path}",
    tag = "proxmox",
    request_body = ProxyParams,
    params(
        ("path" = String, Path, description = "Relative Proxmox JSON path without /api2/json prefix")
    ),
    responses(
        (status = 200, description = "Raw result", body = ExecuteProxmoxResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn proxy_json_post(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ProxyParams>,
    body: Option<Json<Value>>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    let params = merge_params(query.params, body.map(|v| v.0));
    proxy_call(state, "create", path, params).await
}

#[utoipa::path(
    put,
    path = "/api/v1/proxmox/json/{path}",
    tag = "proxmox",
    request_body = ProxyParams,
    params(
        ("path" = String, Path, description = "Relative Proxmox JSON path without /api2/json prefix")
    ),
    responses(
        (status = 200, description = "Raw result", body = ExecuteProxmoxResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn proxy_json_put(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ProxyParams>,
    body: Option<Json<Value>>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    let params = merge_params(query.params, body.map(|v| v.0));
    proxy_call(state, "set", path, params).await
}

#[utoipa::path(
    delete,
    path = "/api/v1/proxmox/json/{path}",
    tag = "proxmox",
    request_body = ProxyParams,
    params(
        ("path" = String, Path, description = "Relative Proxmox JSON path without /api2/json prefix")
    ),
    responses(
        (status = 200, description = "Raw result", body = ExecuteProxmoxResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn proxy_json_delete(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ProxyParams>,
    body: Option<Json<Value>>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    let params = merge_params(query.params, body.map(|v| v.0));
    proxy_call(state, "delete", path, params).await
}

#[utoipa::path(
    get,
    path = "/api/v1/proxmox/extjs/{path}",
    tag = "proxmox",
    params(
        ("path" = String, Path, description = "Relative Proxmox EXTJS path without /api2/extjs prefix")
    ),
    responses(
        (status = 200, description = "Raw result", body = ExecuteProxmoxResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn proxy_extjs_get(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ProxyParams>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    proxy_call(state, "get", path, query.params).await
}

#[utoipa::path(
    post,
    path = "/api/v1/proxmox/extjs/{path}",
    tag = "proxmox",
    request_body = ProxyParams,
    params(
        ("path" = String, Path, description = "Relative Proxmox EXTJS path without /api2/extjs prefix")
    ),
    responses(
        (status = 200, description = "Raw result", body = ExecuteProxmoxResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 500, description = "Backend command failed", body = ErrorResponse)
    )
)]
pub async fn proxy_extjs_post(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ProxyParams>,
    body: Option<Json<Value>>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    let params = merge_params(query.params, body.map(|v| v.0));
    proxy_call(state, "create", path, params).await
}

async fn proxy_call(
    state: AppState,
    method: &str,
    raw_path: String,
    params: BTreeMap<String, String>,
) -> Result<Json<ExecuteProxmoxResponse>, (StatusCode, Json<ErrorResponse>)> {
    let path = normalize_path(&raw_path)?;
    let params = params.into_iter().collect::<Vec<(String, String)>>();
    let result = state
        .proxmox
        .execute(method, &path, &params)
        .await
        .map_err(internal_error)?;

    Ok(Json(ExecuteProxmoxResponse { result }))
}

fn normalize_path(path: &str) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "path cannot be empty".to_string(),
            }),
        ));
    }

    Ok(format!("/{trimmed}"))
}

fn merge_params(mut query: BTreeMap<String, String>, body: Option<Value>) -> BTreeMap<String, String> {
    if let Some(Value::Object(map)) = body {
        for (key, value) in map {
            let converted = match value {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                other => other.to_string(),
            };

            query.insert(key, converted);
        }
    }

    query
}

fn internal_error(err: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}
