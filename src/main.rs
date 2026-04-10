mod auth;
mod config;
mod daemon;
mod dhcp;
mod openapi;
mod proxmox;
mod routes;
mod service_install;
mod vnc_assets;

use std::env;
use std::path::Path;

use config::AppConfig;
use daemon::Daemon;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

fn init_logging(default_level: &str) {
    let normalized_level = normalize_log_level(default_level);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(normalized_level));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn normalize_log_level(value: &str) -> String {
    if value.eq_ignore_ascii_case("info-yapless") {
        return "info".to_owned();
    }

    value.to_owned()
}

fn validate_config(cfg: &AppConfig) -> Result<(), String> {
    if cfg.api.bind == "string"
        || cfg.proxmox.pvesh_bin == "string"
        || cfg.proxmox.pct_bin == "string"
        || cfg.dhcp.bind == "string"
        || cfg.dhcp.server_ip == "string"
        || cfg.dhcp.database_path == "string"
    {
        return Err("configuration contains placeholder 'string' values; update feathertail.toml with real paths/addresses".to_owned());
    }

    if cfg.daemon.poll_interval_secs == 0 {
        return Err("daemon.poll_interval_secs must be > 0".to_owned());
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let local_default = "feathertail.toml";
    let system_default = "/etc/feathertail/feathertail.toml";
    let mut config_path = if Path::new(local_default).exists() {
        local_default.to_owned()
    } else {
        system_default.to_owned()
    };
    let mut run_once = false;
    let mut service_install_mode = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "service-install" => service_install_mode = true,
            "--once" => run_once = true,
            "--config" => {
                if let Some(path) = args.next() {
                    config_path = path;
                } else {
                    eprintln!("missing value for --config");
                    std::process::exit(2);
                }
            }
            value => {
                config_path = value.to_owned();
            }
        }
    }

    if service_install_mode {
        match service_install::install_service(&config_path) {
            Ok(()) => {
                println!("FeatherTail installed and service started");
                return;
            }
            Err(err) => {
                eprintln!("service-install failed: {err}");
                std::process::exit(1);
            }
        }
    }

    let config = match AppConfig::load_or_bootstrap(Path::new(&config_path)) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("failed to load config from {}: {err}", config_path);
            std::process::exit(1);
        }
    };

    if let Err(err) = validate_config(&config) {
        eprintln!("invalid config {}: {err}", config_path);
        std::process::exit(1);
    }

    init_logging(&config.daemon.log_level);

    if !vnc_assets::is_proxmox_host() {
        error!("FeatherTail agent only runs on proxmox hosts");
        eprintln!("FeatherTail agent only runs on proxmox hosts");
        std::process::exit(1);
    }

    if let Err(err) = vnc_assets::install_bundled_vnc_assets() {
        error!(error = %err, "failed to install bundled noVNC assets");
        eprintln!("failed to install bundled noVNC assets: {err}");
        std::process::exit(1);
    }

    info!(target = "/usr/share/novnc-pve", "bundled noVNC assets installed");

    info!(
        name = %config.daemon.name,
        bind = %config.api.bind,
        pvesh = %config.proxmox.pvesh_bin,
        pct = %config.proxmox.pct_bin,
        "starting proxmox daemon"
    );

    let mut daemon = match Daemon::new(config, config_path.clone()).await {
        Ok(d) => d,
        Err(err) => {
            error!(error = %err, "failed to initialize daemon");
            std::process::exit(1);
        }
    };

    let result = if run_once {
        daemon.run_once().await
    } else {
        daemon.run().await
    };

    if let Err(err) = result {
        error!(error = %err, "daemon exited with an error");
        std::process::exit(1);
    }

    info!("daemon stopped cleanly");
}
