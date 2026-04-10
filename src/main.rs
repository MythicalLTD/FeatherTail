mod auth;
mod config;
mod daemon;
mod openapi;
mod proxmox;
mod routes;

use std::env;
use std::path::Path;

use config::AppConfig;
use daemon::Daemon;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

fn init_logging(default_level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level.to_owned()));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() {
    let mut config_path = "feathertail.toml".to_owned();
    let mut run_once = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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

    let config = match AppConfig::load_or_bootstrap(Path::new(&config_path)) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("failed to load config from {}: {err}", config_path);
            std::process::exit(1);
        }
    };

    init_logging(&config.daemon.log_level);

    info!(
        name = %config.daemon.name,
        bind = %config.api.bind,
        pvesh = %config.proxmox.pvesh_bin,
        "starting proxmox daemon"
    );

    let mut daemon = match Daemon::new(config) {
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
