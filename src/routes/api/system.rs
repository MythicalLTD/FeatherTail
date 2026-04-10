use axum::extract::State;
use axum::extract::{Query};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use utoipa::ToSchema;

use crate::config::AppConfig;
use crate::routes::AppState;

const FEATHERTAIL_UPDATE_URL: &str = "https://github.com/MythicalLTD/FeatherTail/releases/latest/download/feathertail-linux-amd64";

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
    pub daemon: String,
}

#[derive(Serialize, ToSchema)]
pub struct StatsResponse {
    pub uptime: Option<u64>,
    pub cpus: Option<u32>,
    pub memory: Option<MemoryStats>,
}

#[derive(Serialize, ToSchema)]
pub struct MemoryStats {
    pub total: u64,
    pub used: u64,
    pub available: Option<u64>,
}

#[derive(Serialize, ToSchema)]
pub struct DiagnosticsResponse {
    pub daemon: String,
    pub config_path: String,
    pub api_bind: String,
    pub dhcp_enabled: bool,
    pub is_proxmox_host: bool,
    pub proxmox_version: Option<String>,
    pub proxmox_error: Option<String>,
    pub now_unix: u64,
}

#[derive(Deserialize, ToSchema)]
pub struct LogsQuery {
    pub lines: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct LogsResponse {
    pub source: String,
    pub lines: usize,
    pub content: String,
}

#[derive(Serialize, ToSchema)]
pub struct ConfigResponse {
    pub config: AppConfig,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateConfigRequest {
    pub config: AppConfig,
    pub restart: Option<bool>,
}

#[derive(Serialize, ToSchema)]
pub struct ActionResponse {
    pub message: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SelfUpdateRequest {
    pub restart: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/api/v1/system/health",
    tag = "system",
    responses(
        (status = 200, description = "Daemon health state", body = HealthResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        daemon: state.daemon_name,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/system/stats",
    tag = "system",
    responses(
        (status = 200, description = "System resource statistics", body = StatsResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse),
        (status = 500, description = "Failed to fetch system stats", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn stats(
    State(_state): State<AppState>,
) -> Result<Json<StatsResponse>, (axum::http::StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    Ok(Json(StatsResponse {
        uptime: None,
        cpus: None,
        memory: None,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/diagnostics",
    tag = "system",
    responses(
        (status = 200, description = "Runtime diagnostics", body = DiagnosticsResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn diagnostics(
    State(state): State<AppState>,
) -> Result<Json<DiagnosticsResponse>, (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    let mut proxmox_version = None;
    let mut proxmox_error = None;

    match state.proxmox.version().await {
        Ok(v) => proxmox_version = Some(v.version),
        Err(err) => proxmox_error = Some(err.to_string()),
    }

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(Json(DiagnosticsResponse {
        daemon: state.daemon_name,
        config_path: state.config_path,
        api_bind: state.api_bind,
        dhcp_enabled: state.dhcp_enabled,
        is_proxmox_host: crate::vnc_assets::is_proxmox_host(),
        proxmox_version,
        proxmox_error,
        now_unix,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/logs",
    tag = "system",
    params(
        ("lines" = Option<usize>, Query, description = "Number of lines to return, default 200")
    ),
    responses(
        (status = 200, description = "Service logs", body = LogsResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse),
        (status = 500, description = "Failed to fetch logs", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn logs(
    Query(query): Query<LogsQuery>,
) -> Result<Json<LogsResponse>, (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    let lines = query.lines.unwrap_or(200).clamp(1, 5000);
    let output = Command::new("journalctl")
        .arg("-u")
        .arg("feathertail")
        .arg("-n")
        .arg(lines.to_string())
        .arg("--no-pager")
        .output()
        .await
        .map_err(internal_error)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(internal_error(stderr));
    }

    Ok(Json(LogsResponse {
        source: "journalctl -u feathertail".to_owned(),
        lines,
        content: String::from_utf8_lossy(&output.stdout).to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/system/config",
    tag = "system",
    responses(
        (status = 200, description = "Current config", body = ConfigResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse),
        (status = 500, description = "Failed to read config", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn get_config(
    State(state): State<AppState>,
) -> Result<Json<ConfigResponse>, (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    let cfg = AppConfig::load_or_bootstrap(Path::new(&state.config_path)).map_err(internal_error)?;
    Ok(Json(ConfigResponse { config: cfg }))
}

#[utoipa::path(
    put,
    path = "/api/v1/system/config",
    tag = "system",
    request_body = UpdateConfigRequest,
    responses(
        (status = 200, description = "Config updated", body = ActionResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse),
        (status = 500, description = "Failed to update config", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    Json(payload): Json<UpdateConfigRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    if let Some(parent) = Path::new(&state.config_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(internal_error)?;
        }
    }

    let serialized = toml::to_string_pretty(&payload.config).map_err(internal_error)?;
    std::fs::write(&state.config_path, serialized).map_err(internal_error)?;

    if payload.restart.unwrap_or(false) {
        schedule_restart();
        return Ok(Json(ActionResponse {
            message: "config updated, restart scheduled".to_owned(),
        }));
    }

    Ok(Json(ActionResponse {
        message: "config updated".to_owned(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/system/restart",
    tag = "system",
    responses(
        (status = 200, description = "Restart scheduled", body = ActionResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn restart() -> Json<ActionResponse> {
    schedule_restart();
    Json(ActionResponse {
        message: "restart scheduled".to_owned(),
    })
}

#[utoipa::path(
    post,
    path = "/api/v1/system/update",
    tag = "system",
    request_body = SelfUpdateRequest,
    responses(
        (status = 200, description = "Update installed", body = ActionResponse),
        (status = 401, description = "Unauthorized", body = crate::routes::api::proxmox::ErrorResponse),
        (status = 500, description = "Update failed", body = crate::routes::api::proxmox::ErrorResponse)
    )
)]
pub async fn self_update(
    Json(payload): Json<SelfUpdateRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    let tmp_path = "/tmp/feathertail.update";
    download_to_file(FEATHERTAIL_UPDATE_URL, tmp_path).await?;

    let current_exe = std::env::current_exe().map_err(internal_error)?;
    std::fs::copy(tmp_path, &current_exe).map_err(internal_error)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&current_exe)
            .map_err(internal_error)?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms).map_err(internal_error)?;
    }

    if payload.restart.unwrap_or(true) {
        schedule_restart();
        return Ok(Json(ActionResponse {
            message: "update installed, restart scheduled".to_owned(),
        }));
    }

    Ok(Json(ActionResponse {
        message: "update installed".to_owned(),
    }))
}

async fn download_to_file(
    url: &str,
    target: &str,
) -> Result<(), (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>)> {
    let curl = Command::new("curl")
        .arg("-fsSL")
        .arg(url)
        .arg("-o")
        .arg(target)
        .output()
        .await;

    if let Ok(output) = curl {
        if output.status.success() {
            return Ok(());
        }
    }

    let wget = Command::new("wget")
        .arg("-q")
        .arg("-O")
        .arg(target)
        .arg(url)
        .output()
        .await
        .map_err(internal_error)?;

    if !wget.status.success() {
        let stderr = String::from_utf8_lossy(&wget.stderr).trim().to_owned();
        return Err(internal_error(format!("download failed: {stderr}")));
    }

    Ok(())
}

fn schedule_restart() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        std::process::exit(0);
    });
}

fn internal_error(
    err: impl std::fmt::Display,
) -> (StatusCode, Json<crate::routes::api::proxmox::ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(crate::routes::api::proxmox::ErrorResponse {
            error: err.to_string(),
        }),
    )
}
