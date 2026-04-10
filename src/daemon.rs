use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use tokio::select;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use tracing::{error, info, warn};

use crate::config::AppConfig;
use crate::proxmox::ProxmoxClient;
use crate::routes;

type DynError = Box<dyn Error + Send + Sync>;

pub struct Daemon {
    config: AppConfig,
    proxmox: Arc<ProxmoxClient>,
}

impl Daemon {
    pub fn new(config: AppConfig) -> Result<Self, DynError> {
        let proxmox = Arc::new(ProxmoxClient::new(&config.proxmox));
        Ok(Self { config, proxmox })
    }

    pub async fn run_once(&mut self) -> Result<(), DynError> {
        match self.proxmox.version().await {
            Ok(version) => {
                info!(
                    version = %version.version,
                    release = %version.release,
                    repoid = %version.repoid,
                    "proxmox health check succeeded"
                );
            }
            Err(err) => {
                warn!(error = %err, "proxmox health check failed");
            }
        }

        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), DynError> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let listener = tokio::net::TcpListener::bind(&self.config.api.bind).await?;

        let app = routes::build_app(routes::AppState {
            daemon_name: self.config.daemon.name.clone(),
            auth_token: self.config.auth.api_token.clone(),
            proxmox: Arc::clone(&self.proxmox),
        });

        let api_task = tokio::spawn(async move {
            let shutdown = wait_for_shutdown(shutdown_rx);
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
        });

        info!(bind = %self.config.api.bind, "api server started");

        let mut ticker = tokio::time::interval(Duration::from_secs(self.config.daemon.poll_interval_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            select! {
                _ = ticker.tick() => {
                    if let Err(err) = self.run_once().await {
                        error!(error = %err, "daemon tick failed");
                    }
                }
                _ = shutdown_signal() => {
                    info!("shutdown signal received");
                    break;
                }
            }
        }

        let _ = shutdown_tx.send(true);

        match api_task.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(Box::new(err)),
            Err(err) => return Err(Box::new(err)),
        }

        Ok(())
    }
}

async fn wait_for_shutdown(mut rx: watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            break;
        }

        if rx.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).expect("failed to listen for SIGTERM");

    select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
