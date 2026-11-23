//! Endpoints backing the built-in admin web UI (`/ui`). Also usable with the bearer token.
//!
//! Everything here is driven by admin input: FeatherTail never creates bridges or changes host
//! networking. A "pool IP" is just "this IP is handed out on this bridge with this gateway".

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::dhcp::{AssignLeaseInput, DhcpLease, PoolIp, expand_ip_list, normalize_mac};
use crate::inventory::{self, Guest, GuestNic, HostBridge};
use crate::routes::AppState;

type ApiError = (StatusCode, Json<serde_json::Value>);
type ApiResult<T> = Result<Json<T>, ApiError>;

fn err(status: StatusCode, message: impl std::fmt::Display) -> ApiError {
    (status, Json(serde_json::json!({ "error": message.to_string() })))
}

fn classify(e: impl std::fmt::Display) -> ApiError {
    let message = e.to_string();
    if message.starts_with("invalid")
        || message.contains("must be")
        || message.contains("not on the same /")
        || message.contains("cannot be combined under")
        || message.contains("too large")
    {
        return err(StatusCode::BAD_REQUEST, message);
    }
    err(StatusCode::INTERNAL_SERVER_ERROR, message)
}

// ---------------------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct BridgeSummary {
    pub name: String,
    /// Exists on this host right now (from /sys/class/net).
    pub present: bool,
    pub state: Option<String>,
    pub host_addresses: Vec<String>,
    pub pool_total: usize,
    pub pool_used: usize,
    pub pool_free: usize,
    pub guest_nics: usize,
    pub guest_nics_without_lease: usize,
}

#[derive(Serialize)]
pub struct Overview {
    pub daemon: String,
    pub version: &'static str,
    pub hostname: Option<String>,
    pub api_bind: String,
    pub dhcp_listener_enabled: bool,
    pub dhcp_server_ip: String,
    pub dhcp_bind: String,
    pub lease_count: usize,
    pub active_lease_count: usize,
    pub pool_count: usize,
    pub guest_count: usize,
    pub nics_without_lease: usize,
    pub bridges: Vec<BridgeSummary>,
    pub password_configured: bool,
    pub token_is_default: bool,
}

pub async fn overview(State(state): State<AppState>) -> ApiResult<Overview> {
    let leases = state.store.list_leases().await.map_err(classify)?;
    let pool = state.store.list_pool().await.map_err(classify)?;
    let guests = inventory::list_guests(&state.pvesh_bin).await;
    let host_bridges = inventory::list_host_bridges().await;
    let bridges = summarize_bridges(&host_bridges, &pool, &leases, &guests);

    let lease_macs: std::collections::HashSet<&str> = leases.iter().map(|l| l.mac.as_str()).collect();
    let nics_without_lease = guests
        .iter()
        .filter(|g| !g.template)
        .flat_map(|g| g.nics.iter())
        .filter(|n| !lease_macs.contains(n.mac.as_str()))
        .filter(|n| {
            // Only count NICs on bridges where the admin actually defined IPs.
            n.bridge
                .as_deref()
                .map(|b| pool.iter().any(|p| p.bridge == b))
                .unwrap_or(false)
        })
        .count();

    let cfg = &state.dhcp_config;
    Ok(Json(Overview {
        daemon: state.daemon_name.clone(),
        version: env!("CARGO_PKG_VERSION"),
        hostname: std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_owned()),
        api_bind: state.api_bind.clone(),
        dhcp_listener_enabled: state.dhcp_enabled,
        dhcp_server_ip: cfg.server_ip.clone(),
        dhcp_bind: cfg.bind.clone(),
        lease_count: leases.len(),
        active_lease_count: leases.iter().filter(|l| l.state == "active").count(),
        pool_count: pool.len(),
        guest_count: guests.iter().filter(|g| !g.template).count(),
        nics_without_lease,
        bridges,
        password_configured: state.auth.has_password(),
        token_is_default: state.token_is_default,
    }))
}

fn summarize_bridges(
    host: &[HostBridge],
    pool: &[PoolIp],
    leases: &[DhcpLease],
    guests: &[Guest],
) -> Vec<BridgeSummary> {
    let leased_ips: std::collections::HashSet<&str> = leases.iter().map(|l| l.ip.as_str()).collect();
    let lease_macs: std::collections::HashSet<&str> = leases.iter().map(|l| l.mac.as_str()).collect();

    let mut names: BTreeMap<String, ()> = BTreeMap::new();
    for b in host {
        names.insert(b.name.clone(), ());
    }
    for p in pool {
        names.insert(p.bridge.clone(), ());
    }

    names
        .into_keys()
        .map(|name| {
            let hb = host.iter().find(|b| b.name == name);
            let entries: Vec<&PoolIp> = pool.iter().filter(|p| p.bridge == name).collect();
            let used = entries.iter().filter(|p| leased_ips.contains(p.ip.as_str())).count();
            let nics: Vec<&GuestNic> = guests
                .iter()
                .filter(|g| !g.template)
                .flat_map(|g| g.nics.iter())
                .filter(|n| n.bridge.as_deref() == Some(name.as_str()))
                .collect();
            BridgeSummary {
                present: hb.is_some(),
                state: hb.and_then(|b| b.state.clone()),
                host_addresses: hb.map(|b| b.addresses.clone()).unwrap_or_default(),
                pool_total: entries.len(),
                pool_used: used,
                pool_free: entries.len() - used,
                guest_nics: nics.len(),
                guest_nics_without_lease: nics.iter().filter(|n| !lease_macs.contains(n.mac.as_str())).count(),
                name,
            }
        })
        .collect()
}

pub async fn bridges(State(state): State<AppState>) -> ApiResult<Vec<BridgeSummary>> {
    let leases = state.store.list_leases().await.map_err(classify)?;
    let pool = state.store.list_pool().await.map_err(classify)?;
    let guests = inventory::list_guests(&state.pvesh_bin).await;
    let host = inventory::list_host_bridges().await;
    Ok(Json(summarize_bridges(&host, &pool, &leases, &guests)))
}

// ---------------------------------------------------------------------------------------
// Guests
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct NicView {
    #[serde(flatten)]
    pub nic: GuestNic,
    pub lease: Option<DhcpLease>,
    /// The admin defined pool IPs on this NIC's bridge.
    pub bridge_has_pool: bool,
    pub free_ips_on_bridge: usize,
}

#[derive(Serialize)]
pub struct GuestView {
    pub vmid: u32,
    pub kind: String,
    pub name: String,
    pub node: String,
    pub status: Option<String>,
    pub template: bool,
    pub nics: Vec<NicView>,
}

pub async fn guests(State(state): State<AppState>) -> ApiResult<Vec<GuestView>> {
    let leases = state.store.list_leases().await.map_err(classify)?;
    let pool = state.store.list_pool().await.map_err(classify)?;
    let guests = inventory::list_guests(&state.pvesh_bin).await;

    let by_mac: HashMap<&str, &DhcpLease> = leases.iter().map(|l| (l.mac.as_str(), l)).collect();
    let leased_ips: std::collections::HashSet<&str> = leases.iter().map(|l| l.ip.as_str()).collect();
    let mut free_by_bridge: HashMap<&str, usize> = HashMap::new();
    let mut has_pool: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for p in &pool {
        has_pool.insert(p.bridge.as_str());
        if !leased_ips.contains(p.ip.as_str()) {
            *free_by_bridge.entry(p.bridge.as_str()).or_default() += 1;
        }
    }

    let views = guests
        .into_iter()
        .map(|g| GuestView {
            nics: g
                .nics
                .into_iter()
                .map(|nic| {
                    let bridge = nic.bridge.clone().unwrap_or_default();
                    NicView {
                        lease: by_mac.get(nic.mac.as_str()).map(|l| (*l).clone()),
                        bridge_has_pool: has_pool.contains(bridge.as_str()),
                        free_ips_on_bridge: free_by_bridge.get(bridge.as_str()).copied().unwrap_or(0),
                        nic,
                    }
                })
                .collect(),
            vmid: g.vmid,
            kind: g.kind,
            name: g.name,
            node: g.node,
            status: g.status,
            template: g.template,
        })
        .collect();
    Ok(Json(views))
}

// ---------------------------------------------------------------------------------------
// IP pool
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PoolHolder {
    pub mac: String,
    pub vmid: Option<u32>,
    pub hostname: Option<String>,
    pub state: String,
    pub guest_name: Option<String>,
}

#[derive(Serialize)]
pub struct PoolView {
    #[serde(flatten)]
    pub entry: PoolIp,
    pub lease: Option<PoolHolder>,
}

#[derive(Deserialize)]
pub struct PoolQuery {
    pub bridge: Option<String>,
}

pub async fn list_pool(
    State(state): State<AppState>,
    Query(query): Query<PoolQuery>,
) -> ApiResult<Vec<PoolView>> {
    let leases = state.store.list_leases().await.map_err(classify)?;
    let pool = state.store.list_pool().await.map_err(classify)?;
    let guests = inventory::list_guests(&state.pvesh_bin).await;
    let names: HashMap<u32, &str> = guests.iter().map(|g| (g.vmid, g.name.as_str())).collect();
    let by_ip: HashMap<&str, &DhcpLease> = leases.iter().map(|l| (l.ip.as_str(), l)).collect();

    let views = pool
        .into_iter()
        .filter(|p| query.bridge.as_deref().map(|b| b == p.bridge).unwrap_or(true))
        .map(|entry| PoolView {
            lease: by_ip.get(entry.ip.as_str()).map(|l| PoolHolder {
                mac: l.mac.clone(),
                vmid: l.vmid,
                hostname: l.hostname.clone(),
                state: l.state.clone(),
                guest_name: l.vmid.and_then(|v| names.get(&v).map(|n| (*n).to_owned())),
            }),
            entry,
        })
        .collect();
    Ok(Json(views))
}

#[derive(Deserialize)]
pub struct AddPoolRequest {
    /// "10.0.0.40-10.0.0.60, 10.0.0.70" or "10.0.0.40-60" or one IP per line.
    pub ips: String,
    pub bridge: String,
    pub gateway: String,
    pub cidr: u8,
    #[serde(default)]
    pub dns_servers: Vec<String>,
    #[serde(default)]
    pub server_ip: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Skip IPs that equal the gateway / network / broadcast address instead of failing.
    #[serde(default = "yes")]
    pub skip_reserved: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize)]
pub struct AddPoolResponse {
    pub added: usize,
    pub skipped: Vec<String>,
    pub moved_from_other_bridge: Vec<String>,
}

pub async fn add_pool(
    State(state): State<AppState>,
    Json(req): Json<AddPoolRequest>,
) -> ApiResult<AddPoolResponse> {
    let ips = expand_ip_list(&req.ips).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    if ips.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "no IPs given"));
    }

    let gateway: Ipv4Addr = req
        .gateway
        .trim()
        .parse()
        .map_err(|_| err(StatusCode::BAD_REQUEST, format!("invalid gateway '{}'", req.gateway)))?;
    let mask = if req.cidr == 0 || req.cidr > 32 {
        return Err(err(StatusCode::BAD_REQUEST, "cidr must be between 1 and 32"));
    } else {
        u32::MAX << (32 - u32::from(req.cidr))
    };
    let net = u32::from(gateway) & mask;
    let bcast = net | !mask;

    let existing = state.store.list_pool().await.map_err(classify)?;
    let mut skipped = Vec::new();
    let mut moved = Vec::new();
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ip in ips {
        if !seen.insert(ip.clone()) {
            continue;
        }
        if req.skip_reserved && req.cidr <= 30 {
            let u = ip.parse::<Ipv4Addr>().map(u32::from).unwrap_or(0);
            if u == u32::from(gateway) || u == net || u == bcast {
                skipped.push(ip);
                continue;
            }
        }
        if let Some(prev) = existing.iter().find(|p| p.ip == ip) {
            if prev.bridge != req.bridge.trim() {
                moved.push(format!("{ip} ({} → {})", prev.bridge, req.bridge.trim()));
            }
        }
        entries.push(PoolIp {
            ip,
            bridge: req.bridge.clone(),
            gateway: req.gateway.clone(),
            cidr: req.cidr,
            dns_servers: req.dns_servers.clone(),
            server_ip: req.server_ip.clone(),
            notes: req.notes.clone(),
        });
    }

    let saved = state.store.upsert_pool(entries).await.map_err(classify)?;
    info!(count = saved.len(), bridge = %req.bridge, "admin added pool ips");
    Ok(Json(AddPoolResponse {
        added: saved.len(),
        skipped,
        moved_from_other_bridge: moved,
    }))
}

pub async fn update_pool(
    State(state): State<AppState>,
    Path(ip): Path<String>,
    Json(mut entry): Json<PoolIp>,
) -> ApiResult<PoolIp> {
    entry.ip = ip;
    let mut saved = state.store.upsert_pool(vec![entry]).await.map_err(classify)?;
    Ok(Json(saved.remove(0)))
}

#[derive(Deserialize)]
pub struct DeletePoolQuery {
    #[serde(default)]
    pub force: bool,
}

pub async fn delete_pool(
    State(state): State<AppState>,
    Path(ip): Path<String>,
    Query(q): Query<DeletePoolQuery>,
) -> ApiResult<serde_json::Value> {
    if !q.force {
        let leases = state.store.list_leases().await.map_err(classify)?;
        if let Some(l) = leases.iter().find(|l| l.ip == ip) {
            return Err(err(
                StatusCode::CONFLICT,
                format!(
                    "{ip} is leased to {} (vmid {}); delete the lease first or force",
                    l.mac,
                    l.vmid.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
                ),
            ));
        }
    }
    let removed = state.store.delete_pool_ip(&ip).await.map_err(classify)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

// ---------------------------------------------------------------------------------------
// Leases
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct LeaseView {
    #[serde(flatten)]
    pub lease: DhcpLease,
    pub guest_name: Option<String>,
    pub guest_status: Option<String>,
    pub nic_key: Option<String>,
    pub nic_bridge: Option<String>,
    pub pool_bridge: Option<String>,
}

pub async fn list_leases(State(state): State<AppState>) -> ApiResult<Vec<LeaseView>> {
    let leases = state.store.list_leases().await.map_err(classify)?;
    let pool = state.store.list_pool().await.map_err(classify)?;
    let guests = inventory::list_guests(&state.pvesh_bin).await;
    let pool_by_ip: HashMap<&str, &PoolIp> = pool.iter().map(|p| (p.ip.as_str(), p)).collect();

    let views = leases
        .into_iter()
        .map(|lease| {
            let guest = lease.vmid.and_then(|v| guests.iter().find(|g| g.vmid == v));
            let nic = guests
                .iter()
                .flat_map(|g| g.nics.iter())
                .find(|n| n.mac == lease.mac);
            LeaseView {
                guest_name: guest.map(|g| g.name.clone()),
                guest_status: guest.and_then(|g| g.status.clone()),
                nic_key: nic.map(|n| n.key.clone()),
                nic_bridge: nic.and_then(|n| n.bridge.clone()),
                pool_bridge: pool_by_ip.get(lease.ip.as_str()).map(|p| p.bridge.clone()),
                lease,
            }
        })
        .collect();
    Ok(Json(views))
}

#[derive(Deserialize)]
pub struct CreateLeaseRequest {
    /// Pick a guest; its NIC MAC is resolved automatically.
    pub vmid: Option<u32>,
    /// Explicit NIC MAC (required when no vmid; optional to pick a specific NIC otherwise).
    pub mac: Option<String>,
    pub ip: String,
    pub hostname: Option<String>,
    /// Only needed when the IP is not in the pool.
    pub gateway: Option<String>,
    pub cidr: Option<u8>,
    pub dns_servers: Option<Vec<String>>,
    pub lease_time_secs: Option<u64>,
    /// Allow assigning an IP whose pool bridge differs from the NIC's bridge.
    #[serde(default)]
    pub force: bool,
}

pub async fn create_lease(
    State(state): State<AppState>,
    Json(req): Json<CreateLeaseRequest>,
) -> ApiResult<DhcpLease> {
    let ip: Ipv4Addr = req
        .ip
        .trim()
        .parse()
        .map_err(|_| err(StatusCode::BAD_REQUEST, format!("invalid ip '{}'", req.ip)))?;
    let ip = ip.to_string();
    let pool_entry = state.store.find_pool_ip(&ip).await.map_err(classify)?;

    // Resolve the NIC.
    let (mac, vmid, node, guest_name, nic_bridge) = match req.vmid {
        Some(vmid) => {
            let guest = inventory::find_guest(&state.pvesh_bin, vmid)
                .await
                .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("guest {vmid} not found in /etc/pve")))?;
            let wanted_mac = req
                .mac
                .as_deref()
                .filter(|m| !m.trim().is_empty())
                .map(normalize_mac)
                .transpose()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let nic = match &wanted_mac {
                Some(m) => guest.nics.iter().find(|n| &n.mac == m),
                None => {
                    let pool_bridge = pool_entry.as_ref().map(|p| p.bridge.as_str());
                    guest
                        .nics
                        .iter()
                        .find(|n| pool_bridge.is_some() && n.bridge.as_deref() == pool_bridge)
                        .or_else(|| guest.nics.first())
                }
            }
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("guest {vmid} has no matching network card")))?;
            (
                nic.mac.clone(),
                Some(vmid),
                Some(guest.node.clone()),
                Some(guest.name.clone()),
                nic.bridge.clone(),
            )
        }
        None => {
            let raw = req
                .mac
                .as_deref()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "pick a VM or give a MAC address"))?;
            (normalize_mac(raw).map_err(|e| err(StatusCode::BAD_REQUEST, e))?, None, None, None, None)
        }
    };

    if let (Some(entry), Some(nb)) = (&pool_entry, &nic_bridge) {
        if &entry.bridge != nb && !req.force {
            return Err(err(
                StatusCode::CONFLICT,
                format!(
                    "{ip} is defined on {} but the selected network card is on {nb}",
                    entry.bridge
                ),
            ));
        }
    }

    if let Some(holder) = state.store.lease_holding_ip(&ip, &mac).await.map_err(classify)? {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "{ip} is already leased to {} (vmid {})",
                holder.mac,
                holder.vmid.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
            ),
        ));
    }

    let (gateway, cidr, dns) = match &pool_entry {
        Some(p) => (
            req.gateway.clone().filter(|g| !g.trim().is_empty()).unwrap_or_else(|| p.gateway.clone()),
            req.cidr.unwrap_or(p.cidr),
            req.dns_servers.clone().unwrap_or_else(|| p.dns_servers.clone()),
        ),
        None => (
            req.gateway
                .clone()
                .filter(|g| !g.trim().is_empty())
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("{ip} is not in the IP pool: add it to a bridge first, or give gateway + cidr")))?,
            req.cidr
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "cidr is required for IPs outside the pool"))?,
            req.dns_servers.clone().unwrap_or_default(),
        ),
    };

    let hostname = req
        .hostname
        .clone()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .or(guest_name);

    let lease = state
        .store
        .assign_static_lease(AssignLeaseInput {
            mac,
            ip,
            hostname,
            vmid,
            node,
            gateway,
            cidr,
            dns_servers: dns,
            lease_time_secs: req.lease_time_secs,
            replace_vmid_leases: false,
        })
        .await
        .map_err(classify)?;
    info!(mac = %lease.mac, ip = %lease.ip, vmid = ?lease.vmid, "admin ui assigned lease");
    Ok(Json(lease))
}

pub async fn delete_lease(
    State(state): State<AppState>,
    Path(mac): Path<String>,
) -> ApiResult<serde_json::Value> {
    let removed = state
        .store
        .remove_lease_by_mac(&mac)
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    info!(mac = %mac, removed, "admin ui deleted lease");
    Ok(Json(serde_json::json!({ "removed": removed })))
}

pub async fn renew_lease(
    State(state): State<AppState>,
    Path(mac): Path<String>,
) -> ApiResult<DhcpLease> {
    let lease = state
        .store
        .reactivate_lease_by_mac(&mac)
        .await
        .map_err(|e| err(StatusCode::CONFLICT, e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no lease for {mac}")))?;
    info!(mac = %lease.mac, ip = %lease.ip, "admin ui renewed lease");
    Ok(Json(lease))
}

// ---------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SettingsView {
    pub dhcp_enabled: bool,
    pub dhcp_bind: String,
    pub dhcp_server_ip: String,
    pub dhcp_lease_time_secs: u64,
    pub api_bind: String,
    pub config_path: String,
    pub password_configured: bool,
    /// Values currently loaded by the running daemon (config changes need a restart).
    pub running_dhcp_enabled: bool,
}

pub async fn get_settings(State(state): State<AppState>) -> ApiResult<SettingsView> {
    let cfg = crate::config::AppConfig::load_or_bootstrap(std::path::Path::new(&state.config_path))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(SettingsView {
        dhcp_enabled: cfg.dhcp.enabled,
        dhcp_bind: cfg.dhcp.bind,
        dhcp_server_ip: cfg.dhcp.server_ip,
        dhcp_lease_time_secs: cfg.dhcp.lease_time_secs,
        api_bind: cfg.api.bind,
        config_path: state.config_path.clone(),
        password_configured: state.auth.has_password(),
        running_dhcp_enabled: state.dhcp_enabled,
    }))
}

#[derive(Deserialize)]
pub struct UpdateDhcpSettings {
    pub enabled: bool,
    pub server_ip: String,
    pub lease_time_secs: u64,
}

/// Edits only the [dhcp] keys in the config file, preserving the rest (comments included).
pub async fn update_settings(
    State(state): State<AppState>,
    Json(req): Json<UpdateDhcpSettings>,
) -> ApiResult<serde_json::Value> {
    let server_ip: Ipv4Addr = req
        .server_ip
        .trim()
        .parse()
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid server ip"))?;
    if req.lease_time_secs < 60 {
        return Err(err(StatusCode::BAD_REQUEST, "lease time must be at least 60 seconds"));
    }
    let contents = std::fs::read_to_string(&state.config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = contents
        .parse()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("config parse failed: {e}")))?;
    if !doc.contains_table("dhcp") {
        doc["dhcp"] = toml_edit::table();
    }
    doc["dhcp"]["enabled"] = toml_edit::value(req.enabled);
    doc["dhcp"]["server_ip"] = toml_edit::value(server_ip.to_string());
    doc["dhcp"]["lease_time_secs"] = toml_edit::value(i64::try_from(req.lease_time_secs).unwrap_or(86_400));
    std::fs::write(&state.config_path, doc.to_string())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    info!(enabled = req.enabled, server_ip = %server_ip, "admin ui updated dhcp settings");
    Ok(Json(serde_json::json!({ "ok": true, "restart_required": true })))
}

/// Restarts the service through systemd (works with Restart=on-failure, unlike a clean exit).
pub async fn restart() -> Json<serde_json::Value> {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let ok = tokio::process::Command::new("systemctl")
            .args(["--no-block", "restart", "feathertail"])
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            // Not under systemd (or unit named differently): exit non-zero so a supervisor restarts us.
            std::process::exit(1);
        }
    });
    Json(serde_json::json!({ "ok": true }))
}
