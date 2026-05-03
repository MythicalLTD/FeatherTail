use std::error::Error;
use std::net::Ipv4Addr;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::watch;
use tracing::{debug, info, warn};
use utoipa::ToSchema;

use crate::config::DhcpConfig;

type DynError = Box<dyn Error + Send + Sync>;
const DHCP_MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_NAK: u8 = 6;
const DHCP_ACK: u8 = 5;
const DHCP_RELEASE: u8 = 7;

#[derive(Debug, Clone, serde::Serialize, ToSchema)]
pub struct DhcpLease {
    pub mac: String,
    pub ip: String,
    pub hostname: Option<String>,
    pub vmid: Option<u32>,
    pub node: Option<String>,
    pub gateway: String,
    pub cidr: u8,
    pub dns_servers: Vec<String>,
    pub lease_start: i64,
    pub lease_end: i64,
    pub state: String,
    pub static_lease: bool,
}

#[derive(Debug, Clone)]
pub struct AssignLeaseInput {
    pub mac: String,
    pub ip: String,
    pub hostname: Option<String>,
    pub vmid: Option<u32>,
    pub node: Option<String>,
    pub gateway: String,
    pub cidr: u8,
    pub dns_servers: Vec<String>,
    pub lease_time_secs: Option<u64>,
}

#[derive(Clone)]
pub struct DhcpService {
    config: DhcpConfig,
    pool: SqlitePool,
}

struct ParsedDhcpRequest {
    xid: [u8; 4],
    flags: [u8; 2],
    ciaddr: Ipv4Addr,
    giaddr: Ipv4Addr,
    chaddr: [u8; 16],
    mac: String,
    msg_type: u8,
    requested_ip: Option<Ipv4Addr>,
}

impl DhcpService {
    pub async fn new(config: DhcpConfig) -> Result<Self, DynError> {
        let db_path_value = config.database_path.clone();
        let db_path = Path::new(&db_path_value);
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn_str = format!("sqlite://{}", db_path.display());
        let opts = SqliteConnectOptions::from_str(&conn_str)?.create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;

        let service = Self { config, pool };
        service.ensure_schema().await?;

        info!(
            db_path = %db_path_value,
            enabled = service.config.enabled,
            bind = %service.config.bind,
            "dhcp service initialized"
        );
        Ok(service)
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub async fn list_leases(&self) -> Result<Vec<DhcpLease>, DynError> {
        let rows = sqlx::query(
            "SELECT mac, ip, hostname, vmid, node, gateway, cidr, dns_servers, lease_start, lease_end, state, static_flag FROM dhcp_leases ORDER BY vmid ASC, mac ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut leases = Vec::with_capacity(rows.len());
        for row in rows {
            leases.push(decode_lease_row(&row)?);
        }

        debug!(count = leases.len(), "dhcp leases loaded from database");

        Ok(leases)
    }

    pub async fn find_lease_by_mac(&self, mac: &str) -> Result<Option<DhcpLease>, DynError> {
        let normalized = normalize_mac(mac)?;
        let row = sqlx::query(
            "SELECT mac, ip, hostname, vmid, node, gateway, cidr, dns_servers, lease_start, lease_end, state, static_flag FROM dhcp_leases WHERE mac = ?",
        )
        .bind(&normalized)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(decode_lease_row(&row)?))
    }

    pub async fn assign_static_lease(&self, input: AssignLeaseInput) -> Result<DhcpLease, DynError> {
        let mac = normalize_mac(&input.mac)?;
        let ip_addr = parse_ipv4(&input.ip)?;
        let gateway_addr = parse_ipv4(&input.gateway)?;
        let effective_cidr =
            effective_cidr_for_gateway_and_ip(ip_addr, gateway_addr, input.cidr)?;
        if effective_cidr != input.cidr {
            warn!(
                requested_cidr = input.cidr,
                effective_cidr,
                vmid = ?input.vmid,
                ip = %ip_addr,
                gateway = %gateway_addr,
                "dhcp assign: widened subnet by one bit so gateway and lease IP share one routed network",
            );
        }
        let gateway = gateway_addr.to_string();

        let dns_servers: Vec<String> = input
            .dns_servers
            .iter()
            .map(|dns| parse_ipv4(dns).map(|v| v.to_string()))
            .collect::<Result<Vec<_>, _>>()?;

        let ip = ip_addr.to_string();
        let now = now_unix();
        let lease_time_secs = input.lease_time_secs.unwrap_or(self.config.lease_time_secs);
        let lease_end = now + i64::try_from(lease_time_secs).unwrap_or(86_400);

        if let Some(vmid) = input.vmid {
            sqlx::query("DELETE FROM dhcp_leases WHERE vmid = ?")
                .bind(i64::from(vmid))
                .execute(&self.pool)
                .await?;
        }

        sqlx::query(
            "INSERT INTO dhcp_leases (mac, ip, hostname, vmid, node, gateway, cidr, dns_servers, lease_start, lease_end, state, static_flag, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', 1, ?) \
             ON CONFLICT(mac) DO UPDATE SET \
                 ip = excluded.ip, \
                 hostname = excluded.hostname, \
                 vmid = excluded.vmid, \
                 node = excluded.node, \
                 gateway = excluded.gateway, \
                 cidr = excluded.cidr, \
                 dns_servers = excluded.dns_servers, \
                 lease_start = excluded.lease_start, \
                 lease_end = excluded.lease_end, \
                 state = 'active', \
                 static_flag = 1, \
                 updated_at = excluded.updated_at",
        )
        .bind(&mac)
        .bind(&ip)
        .bind(input.hostname.clone())
        .bind(input.vmid.map(i64::from))
        .bind(input.node.clone())
        .bind(&gateway)
        .bind(i64::from(effective_cidr))
        .bind(serde_json::to_string(&dns_servers)?)
        .bind(now)
        .bind(lease_end)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let lease = DhcpLease {
            mac,
            ip,
            hostname: input.hostname,
            vmid: input.vmid,
            node: input.node,
            gateway,
            cidr: effective_cidr,
            dns_servers,
            lease_start: now,
            lease_end,
            state: "active".to_owned(),
            static_lease: true,
        };

        info!(
            mac = %lease.mac,
            ip = %lease.ip,
            vmid = ?lease.vmid,
            node = ?lease.node,
            hostname = ?lease.hostname,
            gateway = %lease.gateway,
            cidr = lease.cidr,
            lease_end = lease.lease_end,
            "dhcp lease upserted"
        );

        Ok(lease)
    }

    pub async fn remove_lease_by_vmid(&self, vmid: u32) -> Result<bool, DynError> {
        let result = sqlx::query("DELETE FROM dhcp_leases WHERE vmid = ?")
            .bind(i64::from(vmid))
            .execute(&self.pool)
            .await?;

        if result.rows_affected() > 0 {
            info!(vmid, "dhcp lease deleted from database");
        } else {
            debug!(vmid, "no dhcp lease found to delete");
        }

        Ok(result.rows_affected() > 0)
    }

    pub async fn release_lease_by_mac(&self, mac: &str) -> Result<bool, DynError> {
        let normalized = normalize_mac(mac)?;
        let now = now_unix();

        let result = sqlx::query(
            "UPDATE dhcp_leases SET state = 'released', lease_end = ?, updated_at = ? WHERE mac = ?",
        )
        .bind(now)
        .bind(now)
        .bind(&normalized)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() > 0 {
            info!(mac = %normalized, "dhcp lease released by client");
        } else {
            debug!(mac = %normalized, "dhcp release received for unknown mac");
        }

        Ok(result.rows_affected() > 0)
    }

    pub async fn run_listener(&self, mut shutdown_rx: watch::Receiver<bool>) -> Result<(), DynError> {
        if !self.config.enabled {
            info!("dhcp listener disabled by config");
            return Ok(());
        }

        let socket = UdpSocket::bind(&self.config.bind).await?;
        socket.set_broadcast(true)?;
        info!(
            bind = %self.config.bind,
            "dhcp listener started"
        );

        let mut buffer = [0u8; 1500];
        loop {
            select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("dhcp listener shutdown requested");
                        break;
                    }
                }
                recv = socket.recv_from(&mut buffer) => {
                    match recv {
                        Ok((size, src)) => {
                            debug!(size, src = %src, "dhcp packet received");
                            if let Err(err) = self.handle_dhcp_packet(&socket, &buffer[..size], src).await {
                                warn!(error = %err, src = %src, "failed to process dhcp packet");
                            }
                        }
                        Err(err) => {
                            warn!(error = %err, "dhcp socket receive failed");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_dhcp_packet(
        &self,
        socket: &UdpSocket,
        packet: &[u8],
        src: SocketAddr,
    ) -> Result<(), DynError> {
        let Some(request) = parse_dhcp_request(packet) else {
            return Ok(());
        };

        if request.msg_type == DHCP_RELEASE {
            let _ = self.release_lease_by_mac(&request.mac).await?;
            return Ok(());
        }

        let lease = self.find_lease_by_mac(&request.mac).await?;
        let lease_vmid = lease.as_ref().and_then(|l| l.vmid);

        if !self.host_allowed(&request.mac, lease_vmid) {
            let target = resolve_dhcp_target(&request, src);
            if request.msg_type == DHCP_REQUEST {
                let response = build_dhcp_nak_response(&request, parse_ipv4(&self.config.server_ip)?);
                socket.send_to(&response, target).await?;
                info!(mac = %request.mac, vmid = ?lease_vmid, mode = %self.config.firewall_mode, "dhcp request blocked by firewall (nak sent)");
            } else {
                info!(mac = %request.mac, vmid = ?lease_vmid, mode = %self.config.firewall_mode, "dhcp packet blocked by firewall");
            }
            return Ok(());
        }

        let response_type = match request.msg_type {
            DHCP_DISCOVER => DHCP_OFFER,
            DHCP_REQUEST => DHCP_ACK,
            other => {
                debug!(msg_type = other, mac = %request.mac, "unsupported dhcp message type");
                return Ok(());
            }
        };

        if request.msg_type == DHCP_REQUEST {
            match lease.as_ref() {
                None => {
                    let response = build_dhcp_nak_response(&request, parse_ipv4(&self.config.server_ip)?);
                    let target = resolve_dhcp_target(&request, src);
                    socket.send_to(&response, target).await?;
                    info!(mac = %request.mac, target = %target, "dhcp nak sent for unknown lease");
                    return Ok(());
                }
                Some(found) => {
                    if found.state != "active" {
                        let response = build_dhcp_nak_response(&request, parse_ipv4(&self.config.server_ip)?);
                        let target = resolve_dhcp_target(&request, src);
                        socket.send_to(&response, target).await?;
                        info!(mac = %request.mac, state = %found.state, target = %target, "dhcp nak sent for non-active lease");
                        return Ok(());
                    }

                    let expected = parse_ipv4(&found.ip)?;
                    let requested = request
                        .requested_ip
                        .or_else(|| (request.ciaddr != Ipv4Addr::UNSPECIFIED).then_some(request.ciaddr));
                    if let Some(req_ip) = requested {
                        if req_ip != expected {
                            let response = build_dhcp_nak_response(&request, parse_ipv4(&self.config.server_ip)?);
                            let target = resolve_dhcp_target(&request, src);
                            socket.send_to(&response, target).await?;
                            info!(
                                mac = %request.mac,
                                requested_ip = %req_ip,
                                expected_ip = %expected,
                                target = %target,
                                "dhcp nak sent for mismatched requested ip"
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }

        let Some(lease) = lease else {
            debug!(mac = %request.mac, src = %src, "no lease found for dhcp request mac");
            return Ok(());
        };

        if lease.state != "active" {
            debug!(mac = %lease.mac, state = %lease.state, "ignoring dhcp discover for non-active lease");
            return Ok(());
        }

        let response = build_dhcp_response(
            &request,
            &lease,
            response_type,
            parse_ipv4(&self.config.server_ip)?,
        )?;
        let target = resolve_dhcp_target(&request, src);
        socket.send_to(&response, target).await?;

        info!(
            mac = %lease.mac,
            vmid = ?lease.vmid,
            yiaddr = %lease.ip,
            gateway = %lease.gateway,
            msg_type = response_type,
            target = %target,
            "dhcp response sent"
        );

        Ok(())
    }

    fn host_allowed(&self, mac: &str, vmid: Option<u32>) -> bool {
        let mode = self.config.firewall_mode.to_ascii_lowercase();

        let mac_match = |list: &[String]| {
            list.iter()
                .filter_map(|value| normalize_mac(value).ok())
                .any(|value| value == mac)
        };

        let vmid_match = |list: &[u32]| vmid.map(|v| list.contains(&v)).unwrap_or(false);

        match mode.as_str() {
            "off" => true,
            "blocklist" => {
                let blocked = mac_match(&self.config.firewall_deny_macs)
                    || vmid_match(&self.config.firewall_deny_vmids);
                !blocked
            }
            "allowlist" => {
                mac_match(&self.config.firewall_allow_macs)
                    || vmid_match(&self.config.firewall_allow_vmids)
            }
            other => {
                warn!(mode = %other, "unknown dhcp firewall mode; defaulting to allow");
                true
            }
        }
    }

    async fn ensure_schema(&self) -> Result<(), DynError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS dhcp_leases (\
                mac TEXT PRIMARY KEY,\
                ip TEXT NOT NULL,\
                hostname TEXT,\
                vmid INTEGER,\
                node TEXT,\
                gateway TEXT NOT NULL DEFAULT '0.0.0.0',\
                cidr INTEGER NOT NULL DEFAULT 24,\
                dns_servers TEXT NOT NULL DEFAULT '[]',\
                lease_start INTEGER NOT NULL,\
                lease_end INTEGER NOT NULL,\
                state TEXT NOT NULL,\
                static_flag INTEGER NOT NULL DEFAULT 0,\
                updated_at INTEGER NOT NULL\
            )",
        )
        .execute(&self.pool)
        .await?;

        let _ = sqlx::query("ALTER TABLE dhcp_leases ADD COLUMN gateway TEXT NOT NULL DEFAULT '0.0.0.0'")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE dhcp_leases ADD COLUMN cidr INTEGER NOT NULL DEFAULT 24")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE dhcp_leases ADD COLUMN dns_servers TEXT NOT NULL DEFAULT '[]'")
            .execute(&self.pool)
            .await;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_dhcp_leases_vmid ON dhcp_leases(vmid)")
            .execute(&self.pool)
            .await?;

        debug!("dhcp lease table ensured");

        Ok(())
    }
}

fn subnet_mask_u32(cidr: u8) -> Result<u32, DynError> {
    match cidr {
        0 => Ok(0),
        1..=32 => Ok(u32::MAX << u32::from(32_u8.saturating_sub(cidr))),
        _ => Err("cidr must be between 0 and 32".into()),
    }
}

/// Returns `requested_cidr` when gateway and lease IP lie on one network for that mask, or —
/// once — the next broader mask (`requested_cidr - 1`) when callers mis-state a routed `/23` as `/24`.
/// Otherwise returns an error mentioning the largest workable `cidr` below the requested mask.
fn effective_cidr_for_gateway_and_ip(
    ip: Ipv4Addr,
    gateway: Ipv4Addr,
    requested_cidr: u8,
) -> Result<u8, DynError> {
    if requested_cidr > 32 {
        return Err("cidr must be between 0 and 32".into());
    }
    if requested_cidr == 0 {
        return Ok(0);
    }
    let ip_u = ipv4_to_u32(ip);
    let gw_u = ipv4_to_u32(gateway);
    let mask_r = subnet_mask_u32(requested_cidr)?;
    if (ip_u & mask_r) == (gw_u & mask_r) {
        return Ok(requested_cidr);
    }

    let one_looser = requested_cidr.saturating_sub(1);
    if one_looser > 0 {
        let mask_w = subnet_mask_u32(one_looser)?;
        if (ip_u & mask_w) == (gw_u & mask_w) {
            return Ok(one_looser);
        }
    }

    let mut hint: Option<u8> = None;
    for p in (1..requested_cidr).rev() {
        let m = subnet_mask_u32(p)?;
        if (ip_u & m) == (gw_u & m) {
            hint = Some(p);
            break;
        }
    }

    let Some(largest_below) = hint else {
        return Err(
            format!("ip {ip} and gateway {gateway} cannot be combined under /{requested_cidr}; check addresses")
                .into(),
        );
    };
    Err(format!(
        "ip {ip} and gateway {gateway} are not on the same /{requested_cidr} subnet; shortest shared network under your mask is /{largest_below} (try \"cidr\": {largest_below})"
    )
    .into())
}

fn parse_dhcp_request(packet: &[u8]) -> Option<ParsedDhcpRequest> {
    if packet.len() < 240 {
        return None;
    }

    if packet[0] != 1 || packet[1] != 1 || packet[2] != 6 {
        return None;
    }

    if packet[236..240] != DHCP_MAGIC_COOKIE {
        return None;
    }

    let mut chaddr = [0u8; 16];
    chaddr.copy_from_slice(&packet[28..44]);

    let mac = format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        chaddr[0], chaddr[1], chaddr[2], chaddr[3], chaddr[4], chaddr[5]
    );

    let mut xid = [0u8; 4];
    xid.copy_from_slice(&packet[4..8]);

    let mut flags = [0u8; 2];
    flags.copy_from_slice(&packet[10..12]);

    let ciaddr = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
    let giaddr = Ipv4Addr::new(packet[24], packet[25], packet[26], packet[27]);

    let mut idx = 240;
    let mut msg_type = None;
    let mut requested_ip = None;

    while idx < packet.len() {
        let code = packet[idx];
        idx += 1;

        if code == 0 {
            continue;
        }
        if code == 255 {
            break;
        }
        if idx >= packet.len() {
            break;
        }

        let len = packet[idx] as usize;
        idx += 1;
        if idx + len > packet.len() {
            break;
        }

        if code == 53 && len >= 1 {
            msg_type = Some(packet[idx]);
        }
        if code == 50 && len == 4 {
            requested_ip = Some(Ipv4Addr::new(
                packet[idx],
                packet[idx + 1],
                packet[idx + 2],
                packet[idx + 3],
            ));
        }

        idx += len;
    }

    Some(ParsedDhcpRequest {
        xid,
        flags,
        ciaddr,
        giaddr,
        chaddr,
        mac,
        msg_type: msg_type?,
        requested_ip,
    })
}

fn build_dhcp_response(
    request: &ParsedDhcpRequest,
    lease: &DhcpLease,
    response_type: u8,
    server_ip: Ipv4Addr,
) -> Result<Vec<u8>, DynError> {
    let yiaddr = parse_ipv4(&lease.ip)?;
    let gateway = parse_ipv4(&lease.gateway)?;
    let subnet_mask = cidr_to_mask(lease.cidr);

    let now = now_unix();
    let mut lease_time = if lease.lease_end > now {
        lease.lease_end - now
    } else {
        300
    };
    if lease_time < 60 {
        lease_time = 60;
    }
    let lease_u32 = u32::try_from(lease_time).unwrap_or(3600);

    let mut out = vec![0u8; 240];
    out[0] = 2;
    out[1] = 1;
    out[2] = 6;
    out[3] = 0;
    out[4..8].copy_from_slice(&request.xid);
    out[10..12].copy_from_slice(&request.flags);
    out[16..20].copy_from_slice(&yiaddr.octets());
    out[20..24].copy_from_slice(&server_ip.octets());
    out[24..28].copy_from_slice(&request.giaddr.octets());
    out[28..44].copy_from_slice(&request.chaddr);
    out[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE);

    push_option(&mut out, 53, &[response_type]);
    push_option(&mut out, 54, &server_ip.octets());
    push_option(&mut out, 1, &subnet_mask.octets());
    push_option(&mut out, 3, &gateway.octets());

    let dns_raw: Vec<u8> = lease
        .dns_servers
        .iter()
        .filter_map(|dns| parse_ipv4(dns).ok())
        .flat_map(|ip| ip.octets())
        .collect();
    if !dns_raw.is_empty() {
        push_option(&mut out, 6, &dns_raw);
    }

    push_option(&mut out, 51, &lease_u32.to_be_bytes());
    push_option(&mut out, 58, &(lease_u32 / 2).to_be_bytes());
    push_option(&mut out, 59, &(lease_u32.saturating_mul(7) / 8).to_be_bytes());
    out.push(255);

    Ok(out)
}

fn build_dhcp_nak_response(request: &ParsedDhcpRequest, server_ip: Ipv4Addr) -> Vec<u8> {
    let mut out = vec![0u8; 240];
    out[0] = 2;
    out[1] = 1;
    out[2] = 6;
    out[3] = 0;
    out[4..8].copy_from_slice(&request.xid);
    out[10..12].copy_from_slice(&request.flags);
    out[20..24].copy_from_slice(&server_ip.octets());
    out[24..28].copy_from_slice(&request.giaddr.octets());
    out[28..44].copy_from_slice(&request.chaddr);
    out[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE);

    push_option(&mut out, 53, &[DHCP_NAK]);
    push_option(&mut out, 54, &server_ip.octets());
    push_option(&mut out, 56, b"requested lease is not valid");
    out.push(255);

    out
}

fn push_option(out: &mut Vec<u8>, code: u8, value: &[u8]) {
    if value.is_empty() || value.len() > 255 {
        return;
    }

    out.push(code);
    out.push(value.len() as u8);
    out.extend_from_slice(value);
}

fn resolve_dhcp_target(request: &ParsedDhcpRequest, src: SocketAddr) -> SocketAddr {
    if request.giaddr != Ipv4Addr::UNSPECIFIED {
        return SocketAddr::V4(SocketAddrV4::new(request.giaddr, 67));
    }

    if request.ciaddr != Ipv4Addr::UNSPECIFIED {
        return SocketAddr::V4(SocketAddrV4::new(request.ciaddr, 68));
    }

    if let SocketAddr::V4(v4) = src {
        if *v4.ip() != Ipv4Addr::UNSPECIFIED {
            return SocketAddr::V4(SocketAddrV4::new(*v4.ip(), 68));
        }
    }

    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(255, 255, 255, 255), 68))
}

fn cidr_to_mask(cidr: u8) -> Ipv4Addr {
    if cidr == 0 {
        return Ipv4Addr::UNSPECIFIED;
    }

    let mask = u32::MAX << (32 - cidr.min(32));
    Ipv4Addr::from(mask.to_be_bytes())
}

fn ipv4_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from_be_bytes(ip.octets())
}

fn parse_ipv4(value: &str) -> Result<Ipv4Addr, DynError> {
    Ok(value.parse::<Ipv4Addr>()?)
}

fn now_unix() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

pub fn normalize_mac(value: &str) -> Result<String, DynError> {
    let stripped: String = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();

    if stripped.len() != 12 {
        return Err(format!("invalid mac address: {value}").into());
    }

    let mut bytes = [0u8; 6];
    for i in 0..6 {
        let start = i * 2;
        let pair = &stripped[start..start + 2];
        bytes[i] = u8::from_str_radix(pair, 16)?;
    }

    Ok(format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    ))
}

fn decode_lease_row(row: &sqlx::sqlite::SqliteRow) -> Result<DhcpLease, DynError> {
    let vmid_i64: Option<i64> = row.try_get("vmid")?;
    let cidr_i64: i64 = row.try_get("cidr")?;
    let dns_raw: String = row.try_get("dns_servers")?;

    Ok(DhcpLease {
        mac: row.try_get("mac")?,
        ip: row.try_get("ip")?,
        hostname: row.try_get("hostname")?,
        vmid: vmid_i64.and_then(|v| u32::try_from(v).ok()),
        node: row.try_get("node")?,
        gateway: row.try_get("gateway")?,
        cidr: u8::try_from(cidr_i64).map_err(|_| "invalid cidr in lease row")?,
        dns_servers: serde_json::from_str(&dns_raw).unwrap_or_default(),
        lease_start: row.try_get("lease_start")?,
        lease_end: row.try_get("lease_end")?,
        state: row.try_get("state")?,
        static_lease: row.try_get::<i64, _>("static_flag")? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_packet_with_options(options: &[u8]) -> Vec<u8> {
        let mut packet = vec![0u8; 240];
        packet[0] = 1;
        packet[1] = 1;
        packet[2] = 6;
        packet[3] = 0;
        packet[4..8].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        packet[10..12].copy_from_slice(&[0x80, 0x00]);
        packet[28..44].copy_from_slice(&[
            0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        packet[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE);
        packet.extend_from_slice(options);
        packet
    }

    #[test]
    fn normalize_mac_formats_common_inputs() {
        let normalized = normalize_mac("AA-BB-CC-DD-EE-FF").expect("mac should normalize");
        assert_eq!(normalized, "aa:bb:cc:dd:ee:ff");

        let normalized2 = normalize_mac("aabb.ccdd.eeff").expect("mac should normalize");
        assert_eq!(normalized2, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn normalize_mac_rejects_invalid_length() {
        let err = normalize_mac("aa:bb:cc").expect_err("invalid mac should fail");
        assert!(err.to_string().contains("invalid mac address"));
    }

    #[test]
    fn parse_dhcp_request_extracts_message_and_requested_ip() {
        let packet = base_packet_with_options(&[
            53, 1, DHCP_REQUEST, 50, 4, 10, 0, 0, 42, 255,
        ]);

        let parsed = parse_dhcp_request(&packet).expect("packet should parse");
        assert_eq!(parsed.msg_type, DHCP_REQUEST);
        assert_eq!(parsed.mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(parsed.requested_ip, Some(Ipv4Addr::new(10, 0, 0, 42)));
    }

    #[test]
    fn build_dhcp_nak_sets_nak_type_and_server_id() {
        let packet = base_packet_with_options(&[53, 1, DHCP_REQUEST, 255]);
        let parsed = parse_dhcp_request(&packet).expect("packet should parse");
        let server = Ipv4Addr::new(193, 34, 77, 2);

        let nak = build_dhcp_nak_response(&parsed, server);

        assert!(nak.windows(3).any(|w| w == [53, 1, DHCP_NAK]));
        assert!(nak.windows(6).any(|w| w == [54, 4, 193, 34, 77, 2]));
    }

    #[test]
    fn build_dhcp_response_includes_router_dns_and_mask() {
        let packet = base_packet_with_options(&[53, 1, DHCP_DISCOVER, 255]);
        let parsed = parse_dhcp_request(&packet).expect("packet should parse");

        let lease = DhcpLease {
            mac: "aa:bb:cc:dd:ee:ff".to_owned(),
            ip: "193.34.77.10".to_owned(),
            hostname: Some("vm-1".to_owned()),
            vmid: Some(100),
            node: Some("node1".to_owned()),
            gateway: "193.34.77.1".to_owned(),
            cidr: 24,
            dns_servers: vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()],
            lease_start: 0,
            lease_end: i64::MAX,
            state: "active".to_owned(),
            static_lease: true,
        };

        let response = build_dhcp_response(&parsed, &lease, DHCP_OFFER, Ipv4Addr::new(193, 34, 77, 2))
            .expect("response should build");

        assert!(response.windows(3).any(|w| w == [53, 1, DHCP_OFFER]));
        assert!(response.windows(6).any(|w| w == [1, 4, 255, 255, 255, 0]));
        assert!(response.windows(6).any(|w| w == [3, 4, 193, 34, 77, 1]));
        assert!(response
            .windows(10)
            .any(|w| w == [6, 8, 1, 1, 1, 1, 8, 8, 8, 8]));
    }

    #[test]
    fn resolve_dhcp_target_prefers_giaddr_then_ciaddr() {
        let mut packet = base_packet_with_options(&[53, 1, DHCP_REQUEST, 255]);
        packet[12..16].copy_from_slice(&[10, 0, 0, 22]);
        packet[24..28].copy_from_slice(&[10, 0, 0, 1]);
        let parsed = parse_dhcp_request(&packet).expect("packet should parse");

        let src: SocketAddr = "0.0.0.0:68".parse().expect("valid socket");
        let target = resolve_dhcp_target(&parsed, src);
        assert_eq!(target, "10.0.0.1:67".parse::<SocketAddr>().expect("valid socket"));

        let mut packet2 = base_packet_with_options(&[53, 1, DHCP_REQUEST, 255]);
        packet2[12..16].copy_from_slice(&[10, 0, 0, 22]);
        let parsed2 = parse_dhcp_request(&packet2).expect("packet should parse");
        let target2 = resolve_dhcp_target(&parsed2, src);
        assert_eq!(target2, "10.0.0.22:68".parse::<SocketAddr>().expect("valid socket"));
    }

    #[test]
    fn effective_cidr_keeps_matching_gateway_network() {
        let ip = Ipv4Addr::new(193, 34, 77, 50);
        let gw = Ipv4Addr::new(193, 34, 77, 1);
        assert_eq!(
            effective_cidr_for_gateway_and_ip(ip, gw, 24).unwrap(),
            24
        );
    }

    #[test]
    fn effective_cidr_widens_one_bit_when_routed_supernet_matches() {
        let ip = Ipv4Addr::new(212, 87, 213, 115);
        let gw = Ipv4Addr::new(212, 87, 212, 1);
        assert_eq!(
            effective_cidr_for_gateway_and_ip(ip, gw, 24).unwrap(),
            23
        );
    }

    #[test]
    fn effective_cidr_rejects_far_mismatch() {
        let ip = Ipv4Addr::new(193, 34, 78, 50);
        let gw = Ipv4Addr::new(193, 34, 77, 1);
        assert!(effective_cidr_for_gateway_and_ip(ip, gw, 24).is_err());
    }
}
