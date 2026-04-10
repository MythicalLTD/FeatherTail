use std::error::Error;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::select;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info, warn};

use crate::config::AppConfig;
use crate::dhcp::DhcpService;
use crate::proxmox::ProxmoxClient;
use crate::routes;

type DynError = Box<dyn Error + Send + Sync>;

pub struct Daemon {
    config: AppConfig,
    config_path: String,
    proxmox: Arc<ProxmoxClient>,
    dhcp: Arc<DhcpService>,
    seen_qemu_vmids: HashSet<u32>,
    seen_lxc_vmids: HashSet<u32>,
    info_yapless: bool,
}

impl Daemon {
    pub async fn new(config: AppConfig, config_path: String) -> Result<Self, DynError> {
        let proxmox = Arc::new(ProxmoxClient::new(&config.proxmox));
        let dhcp = Arc::new(DhcpService::new(config.dhcp.clone()).await?);
        let info_yapless = config.daemon.log_level.eq_ignore_ascii_case("info-yapless");
        Ok(Self {
            config,
            config_path,
            proxmox,
            dhcp,
            seen_qemu_vmids: HashSet::new(),
            seen_lxc_vmids: HashSet::new(),
            info_yapless,
        })
    }

    pub async fn run_once(&mut self) -> Result<(), DynError> {
        if self.info_yapless {
            debug!("daemon poll cycle started");
        } else {
            info!("daemon poll cycle started");
        }

        match self.proxmox.version().await {
            Ok(version) => {
                if self.info_yapless {
                    debug!(
                        version = %version.version,
                        release = %version.release,
                        repoid = %version.repoid,
                        "proxmox health check succeeded"
                    );
                } else {
                    info!(
                        version = %version.version,
                        release = %version.release,
                        repoid = %version.repoid,
                        "proxmox health check succeeded"
                    );
                }
            }
            Err(err) => {
                warn!(error = %err, "proxmox health check failed");
            }
        }

        match self.proxmox.qemu_list().await {
            Ok(vms) => {
                let mut current = HashSet::new();
                for vm in &vms {
                    current.insert(vm.vmid);
                    if !self.seen_qemu_vmids.contains(&vm.vmid) {
                        if self.info_yapless {
                            debug!(
                                vmid = vm.vmid,
                                node = %vm.node,
                                name = %vm.name,
                                "new qemu vm discovered"
                            );
                        } else {
                            info!(
                                vmid = vm.vmid,
                                node = %vm.node,
                                name = %vm.name,
                                "new qemu vm discovered"
                            );
                        }
                    }
                }
                self.seen_qemu_vmids = current;
                if self.info_yapless {
                    debug!(count = vms.len(), "qemu pull completed");
                } else {
                    info!(count = vms.len(), "qemu pull completed");
                }
            }
            Err(err) => {
                warn!(error = %err, "qemu pull failed");
            }
        }

        match self.proxmox.lxc_list().await {
            Ok(containers) => {
                let mut current = HashSet::new();
                for ct in &containers {
                    current.insert(ct.vmid);
                    if !self.seen_lxc_vmids.contains(&ct.vmid) {
                        if self.info_yapless {
                            debug!(
                                vmid = ct.vmid,
                                node = %ct.node,
                                name = %ct.name,
                                "new lxc container discovered"
                            );
                        } else {
                            info!(
                                vmid = ct.vmid,
                                node = %ct.node,
                                name = %ct.name,
                                "new lxc container discovered"
                            );
                        }
                    }
                }
                self.seen_lxc_vmids = current;
                if self.info_yapless {
                    debug!(count = containers.len(), "lxc pull completed");
                } else {
                    info!(count = containers.len(), "lxc pull completed");
                }
            }
            Err(err) => {
                warn!(error = %err, "lxc pull failed");
            }
        }

        if self.info_yapless {
            debug!("daemon poll cycle completed");
        } else {
            info!("daemon poll cycle completed");
        }

        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), DynError> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let listener = tokio::net::TcpListener::bind(&self.config.api.bind).await?;

        let app = routes::build_app(routes::AppState {
            daemon_name: self.config.daemon.name.clone(),
            auth_token: self.config.auth.api_token.clone(),
            config_path: self.config_path.clone(),
            api_bind: self.config.api.bind.clone(),
            dhcp_enabled: self.dhcp.enabled(),
            proxmox: Arc::clone(&self.proxmox),
            dhcp: if self.dhcp.enabled() {
                Some(Arc::clone(&self.dhcp))
            } else {
                None
            },
        });

        let api_task = tokio::spawn(async move {
            let shutdown = wait_for_shutdown(shutdown_rx);
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
        });

        info!(bind = %self.config.api.bind, "api server started");

        let dhcp_shutdown_rx = shutdown_tx.subscribe();
        let dhcp = Arc::clone(&self.dhcp);
        let dhcp_task = tokio::spawn(async move { dhcp.run_listener(dhcp_shutdown_rx).await });

        let mut ticker = tokio::time::interval(Duration::from_secs(self.config.daemon.poll_interval_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            select! {
                _ = ticker.tick() => {
                    select! {
                        result = self.run_once() => {
                            if let Err(err) = result {
                                error!(error = %err, "daemon tick failed");
                            }
                        }
                        _ = shutdown_signal() => {
                            info!("shutdown signal received");
                            break;
                        }
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

        match dhcp_task.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
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
