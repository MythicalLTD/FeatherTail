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

        if self.config.proxmox.sync_ip_tags {
            if let Err(err) = self.sync_ip_tags().await {
                warn!(error = %err, "proxmox ip tag sync failed");
            }
        }

        if self.info_yapless {
            debug!("daemon poll cycle completed");
        } else {
            info!("daemon poll cycle completed");
        }

        Ok(())
    }

    /// Puts every guest's DHCP lease IPs on it as Proxmox tags and removes the ones
    /// FeatherTail added earlier whose lease is gone. Hand-made tags are never touched.
    async fn sync_ip_tags(&self) -> Result<(), DynError> {
        let leases = self.dhcp.list_leases().await?;
        let mut managed = self.dhcp.managed_ip_tags().await?;
        let guests = crate::inventory::list_guests(&self.config.proxmox.pvesh_bin).await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();

        let live: Vec<_> = leases
            .iter()
            .filter(|l| l.static_lease || (l.state == "active" && l.lease_end > now))
            .collect();

        for guest in guests.iter().filter(|g| !g.template) {
            let macs: HashSet<&str> = guest.nics.iter().map(|n| n.mac.as_str()).collect();
            let desired: Vec<String> = {
                let mut ips: Vec<String> = live
                    .iter()
                    .filter(|l| l.vmid == Some(guest.vmid) || macs.contains(l.mac.as_str()))
                    .map(|l| l.ip.clone())
                    .collect();
                ips.sort();
                ips.dedup();
                ips
            };
            let previously = managed.remove(&guest.vmid).unwrap_or_default();
            if desired.is_empty() && previously.is_empty() {
                continue;
            }

            let (new_tags, new_managed) = plan_ip_tags(&guest.tags, &previously, &desired);
            if new_tags != guest.tags {
                if let Err(err) = self
                    .proxmox
                    .set_guest_tags(&guest.node, &guest.kind, guest.vmid, &new_tags)
                    .await
                {
                    warn!(vmid = guest.vmid, error = %err, "failed to update proxmox ip tags");
                    continue;
                }
                info!(vmid = guest.vmid, tags = %new_tags.join(";"), "proxmox ip tags updated");
            }
            if new_managed != previously {
                self.dhcp.set_managed_ip_tags(guest.vmid, &new_managed).await?;
            }
        }

        // Guests that no longer exist: forget their tags.
        for vmid in managed.into_keys() {
            self.dhcp.set_managed_ip_tags(vmid, &HashSet::new()).await?;
        }

        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), DynError> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let listener = tokio::net::TcpListener::bind(&self.config.api.bind).await?;

        let app = routes::build_app(routes::AppState {
            daemon_name: self.config.daemon.name.clone(),
            store: Arc::clone(&self.dhcp),
            auth: Arc::new(crate::auth::AdminAuth::new(
                self.config.auth.api_token.clone(),
                self.config.auth.admin_password_hash.clone(),
                self.config_path.clone(),
            )),
            pvesh_bin: self.config.proxmox.pvesh_bin.clone(),
            dhcp_config: self.config.dhcp.clone(),
            token_is_default: {
                let t = self.config.auth.api_token.trim();
                t.is_empty() || t == "change-me"
            },
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

/// Returns the guest's new tag list and the set of tags FeatherTail now owns on it.
/// Tags the user added by hand stay, even when they look like an IP.
fn plan_ip_tags(
    current: &[String],
    managed: &HashSet<String>,
    desired: &[String],
) -> (Vec<String>, HashSet<String>) {
    let desired_set: HashSet<&str> = desired.iter().map(String::as_str).collect();
    let mut tags: Vec<String> = current
        .iter()
        .filter(|t| !managed.contains(*t) || desired_set.contains(t.as_str()))
        .cloned()
        .collect();

    let mut owned = HashSet::new();
    for ip in desired {
        let present = tags.iter().any(|t| t.eq_ignore_ascii_case(ip));
        if !present {
            tags.push(ip.clone());
            owned.insert(ip.clone());
        } else if managed.contains(ip) {
            owned.insert(ip.clone());
        }
    }
    (tags, owned)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn adds_missing_ips_and_keeps_user_tags() {
        let (tags, owned) = plan_ip_tags(&v(&["prod"]), &set(&[]), &v(&["10.0.0.5"]));
        assert_eq!(tags, v(&["prod", "10.0.0.5"]));
        assert_eq!(owned, set(&["10.0.0.5"]));
    }

    #[test]
    fn removes_only_stale_managed_ips() {
        let (tags, owned) = plan_ip_tags(
            &v(&["prod", "10.0.0.5", "192.168.1.9"]),
            &set(&["10.0.0.5"]),
            &v(&["10.0.0.6"]),
        );
        assert_eq!(tags, v(&["prod", "192.168.1.9", "10.0.0.6"]));
        assert_eq!(owned, set(&["10.0.0.6"]));
    }

    #[test]
    fn does_not_claim_hand_made_ip_tag() {
        let (tags, owned) = plan_ip_tags(&v(&["10.0.0.5"]), &set(&[]), &v(&["10.0.0.5"]));
        assert_eq!(tags, v(&["10.0.0.5"]));
        assert!(owned.is_empty());
    }

    #[test]
    fn no_change_when_in_sync() {
        let current = v(&["web", "10.0.0.5"]);
        let (tags, owned) = plan_ip_tags(&current, &set(&["10.0.0.5"]), &v(&["10.0.0.5"]));
        assert_eq!(tags, current);
        assert_eq!(owned, set(&["10.0.0.5"]));
    }
}
