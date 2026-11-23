use std::collections::HashSet;
use std::error::Error;
use std::net::Ipv4Addr;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tokio::select;
use tokio::sync::watch;
use tracing::{debug, info, warn};
use utoipa::ToSchema;

use crate::config::{DhcpConfig, DhcpRange};
use crate::dhcp_socket::{DhcpSocket, interface_name};

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
    /// FeatherPanel semantics: one lease per VM, so older leases of the same vmid are dropped.
    /// The admin UI sets this to false so multi-NIC VMs keep one lease per MAC.
    pub replace_vmid_leases: bool,
}

/// One IP the admin put on a bridge. Purely user input: FeatherTail never touches host
/// networking, it only answers DHCP on that bridge with these values.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PoolIp {
    pub ip: String,
    pub bridge: String,
    pub gateway: String,
    pub cidr: u8,
    #[serde(default)]
    pub dns_servers: Vec<String>,
    /// DHCP server identifier sent to clients. Defaults to the gateway when empty.
    #[serde(default)]
    pub server_ip: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl PoolIp {
    fn server_identifier(&self) -> Option<Ipv4Addr> {
        self.server_ip
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .and_then(|v| v.parse().ok())
            .or_else(|| self.gateway.parse().ok())
    }

    /// Validates and normalizes user input.
    pub fn validate(&mut self) -> Result<(), DynError> {
        self.bridge = self.bridge.trim().to_owned();
        validate_bridge_name(&self.bridge)?;
        let ip = parse_ipv4(self.ip.trim())
            .map_err(|_| format!("invalid ip '{}'", self.ip.trim()))?;
        let gateway = parse_ipv4(self.gateway.trim())
            .map_err(|_| format!("invalid gateway '{}'", self.gateway.trim()))?;
        if ip == gateway {
            return Err(format!("invalid ip {ip}: it is the gateway itself").into());
        }
        let cidr = effective_cidr_for_gateway_and_ip(ip, gateway, self.cidr)?;
        if cidr != self.cidr {
            return Err(format!(
                "invalid ip {ip}: not in the same /{} as gateway {gateway} (try /{cidr})",
                self.cidr
            )
            .into());
        }
        self.ip = ip.to_string();
        self.gateway = gateway.to_string();
        self.server_ip = match self.server_ip.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(raw) => Some(
                parse_ipv4(raw)
                    .map_err(|_| format!("invalid server ip '{raw}'"))?
                    .to_string(),
            ),
        };
        self.dns_servers = normalize_dns_list(&self.dns_servers)?;
        self.notes = self
            .notes
            .take()
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty());
        Ok(())
    }
}

pub fn validate_bridge_name(bridge: &str) -> Result<(), DynError> {
    if bridge.is_empty()
        || bridge.len() > 15
        || !bridge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(format!("invalid bridge '{bridge}': use an interface name like vmbr1").into());
    }
    Ok(())
}

fn normalize_dns_list(values: &[String]) -> Result<Vec<String>, DynError> {
    values
        .iter()
        .map(|d| d.trim())
        .filter(|d| !d.is_empty())
        .map(|d| {
            parse_ipv4(d)
                .map(|v| v.to_string())
                .map_err(|_| -> DynError { format!("invalid dns server '{d}'").into() })
        })
        .collect()
}

/// Expands "a.b.c.d-a.b.c.e" / "a.b.c.d - e" / single IPs / comma or newline lists.
pub fn expand_ip_list(input: &str) -> Result<Vec<String>, DynError> {
    let mut out = Vec::new();
    for token in input.split([',', ';', '\n']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((a, b)) = token.split_once('-') {
            let start = parse_ipv4(a.trim()).map_err(|_| format!("invalid ip '{}'", a.trim()))?;
            let b = b.trim();
            let end = if b.contains('.') {
                parse_ipv4(b).map_err(|_| format!("invalid ip '{b}'"))?
            } else {
                let last: u8 = b.parse().map_err(|_| format!("invalid range end '{b}'"))?;
                let o = start.octets();
                Ipv4Addr::new(o[0], o[1], o[2], last)
            };
            let (s, e) = (ipv4_to_u32(start), ipv4_to_u32(end));
            if s > e {
                return Err(format!("invalid range {token}: start is after end").into());
            }
            if e - s >= 1024 {
                return Err(format!("range {token} is too large (max 1024 IPs at once)").into());
            }
            for u in s..=e {
                out.push(Ipv4Addr::from(u).to_string());
            }
        } else {
            out.push(parse_ipv4(token).map_err(|_| format!("invalid ip '{token}'"))?.to_string());
        }
    }
    out.dedup();
    Ok(out)
}

/// Seconds an offered (not yet requested) dynamic IP stays reserved for the client.
const DYNAMIC_OFFER_HOLD_SECS: i64 = 60;

/// A validated `[[dhcp.ranges]]` entry.
#[derive(Debug, Clone)]
struct DynamicRange {
    interface: Option<String>,
    start: u32,
    end: u32,
    gateway: Ipv4Addr,
    cidr: u8,
    dns_servers: Vec<String>,
    server_ip: Ipv4Addr,
    lease_time_secs: u64,
}

impl DynamicRange {
    fn from_config(raw: &DhcpRange, default_lease_secs: u64) -> Result<Self, DynError> {
        let interface = match raw.interface.trim() {
            "" => None,
            name => {
                validate_bridge_name(name)?;
                Some(name.to_owned())
            }
        };
        let start = parse_ipv4(raw.start.trim())
            .map_err(|_| format!("invalid range start '{}'", raw.start.trim()))?;
        let end = parse_ipv4(raw.end.trim())
            .map_err(|_| format!("invalid range end '{}'", raw.end.trim()))?;
        let gateway = parse_ipv4(raw.gateway.trim())
            .map_err(|_| format!("invalid range gateway '{}'", raw.gateway.trim()))?;
        let mask = subnet_mask_u32(raw.cidr)?;
        let (s, e) = (ipv4_to_u32(start), ipv4_to_u32(end));
        if s > e {
            return Err(format!("invalid range {start}-{end}: start is after end").into());
        }
        if s & mask != e & mask {
            return Err(format!(
                "invalid range {start}-{end}: start and end are not in the same /{}",
                raw.cidr
            )
            .into());
        }
        let server_ip = match raw.server_ip.as_deref().map(str::trim) {
            None | Some("") => gateway,
            Some(v) => parse_ipv4(v).map_err(|_| format!("invalid range server_ip '{v}'"))?,
        };
        Ok(Self {
            interface,
            start: s,
            end: e,
            gateway,
            cidr: raw.cidr,
            dns_servers: normalize_dns_list(&raw.dns_servers)?,
            server_ip,
            lease_time_secs: raw.lease_time_secs.unwrap_or(default_lease_secs).max(60),
        })
    }

    fn serves_interface(&self, ifname: Option<&str>) -> bool {
        match (&self.interface, ifname) {
            (None, _) | (_, None) => true,
            (Some(want), Some(got)) => want == got,
        }
    }

    fn contains(&self, ip: Ipv4Addr) -> bool {
        (self.start..=self.end).contains(&ipv4_to_u32(ip))
    }

    /// Network, broadcast, gateway and server addresses are never handed out.
    fn assignable(&self, ip: Ipv4Addr) -> bool {
        let host_bits = !subnet_mask_u32(self.cidr).unwrap_or(u32::MAX);
        let u = ipv4_to_u32(ip);
        let net_or_bcast = self.cidr < 31 && (u & host_bits == 0 || u & host_bits == host_bits);
        self.contains(ip) && !net_or_bcast && ip != self.gateway && ip != self.server_ip
    }
}

/// Checks a `[[dhcp.ranges]]` entry the same way the daemon does at startup.
pub fn validate_range(raw: &DhcpRange) -> Result<(), String> {
    DynamicRange::from_config(raw, 86_400)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Clone)]
pub struct DhcpService {
    config: DhcpConfig,
    pool: SqlitePool,
    ranges: Vec<DynamicRange>,
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

        let mut ranges = Vec::new();
        for raw in &config.ranges {
            match DynamicRange::from_config(raw, config.lease_time_secs) {
                Ok(range) => {
                    info!(
                        interface = ?range.interface,
                        start = %Ipv4Addr::from(range.start),
                        end = %Ipv4Addr::from(range.end),
                        gateway = %range.gateway,
                        "dhcp dynamic range loaded"
                    );
                    ranges.push(range);
                }
                Err(err) => warn!(error = %err, "ignoring invalid [[dhcp.ranges]] entry"),
            }
        }

        let service = Self { config, pool, ranges };
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

        if let (true, Some(vmid)) = (input.replace_vmid_leases, input.vmid) {
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

    /// Extends an active lease in the database when a client ACKs (renew/rebind).
    pub async fn renew_lease_by_mac(&self, mac: &str) -> Result<Option<DhcpLease>, DynError> {
        let normalized = normalize_mac(mac)?;
        let now = now_unix();
        let lease_end =
            now + i64::try_from(self.config.lease_time_secs).unwrap_or(86_400);

        let result = sqlx::query(
            "UPDATE dhcp_leases SET lease_start = ?, lease_end = ?, state = 'active', updated_at = ? \
             WHERE mac = ? AND state = 'active'",
        )
        .bind(now)
        .bind(lease_end)
        .bind(now)
        .bind(&normalized)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        debug!(mac = %normalized, lease_end, "dhcp lease renewed in database");
        self.find_lease_by_mac(&normalized).await
    }

    pub async fn remove_lease_by_mac(&self, mac: &str) -> Result<bool, DynError> {
        let normalized = normalize_mac(mac)?;
        let result = sqlx::query("DELETE FROM dhcp_leases WHERE mac = ?")
            .bind(&normalized)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() > 0 {
            info!(mac = %normalized, "dhcp lease deleted from database");
        }
        Ok(result.rows_affected() > 0)
    }

    /// Returns the lease (other than `except_mac`) that already holds `ip`, if any.
    pub async fn lease_holding_ip(&self, ip: &str, except_mac: &str) -> Result<Option<DhcpLease>, DynError> {
        let except = normalize_mac(except_mac)?;
        let row = sqlx::query(
            "SELECT mac, ip, hostname, vmid, node, gateway, cidr, dns_servers, lease_start, lease_end, state, static_flag FROM dhcp_leases WHERE ip = ? AND mac != ? LIMIT 1",
        )
        .bind(ip)
        .bind(&except)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| decode_lease_row(&r)).transpose()
    }

    // ---- IP pool (admin UI) -------------------------------------------------------------

    pub async fn list_pool(&self) -> Result<Vec<PoolIp>, DynError> {
        let rows = sqlx::query(
            "SELECT ip, bridge, gateway, cidr, dns_servers, server_ip, notes FROM dhcp_pool",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(decode_pool_row(&row)?);
        }
        out.sort_by(|a, b| {
            a.bridge.cmp(&b.bridge).then_with(|| {
                let ua = a.ip.parse::<Ipv4Addr>().map(ipv4_to_u32).unwrap_or(0);
                let ub = b.ip.parse::<Ipv4Addr>().map(ipv4_to_u32).unwrap_or(0);
                ua.cmp(&ub)
            })
        });
        Ok(out)
    }

    pub async fn find_pool_ip(&self, ip: &str) -> Result<Option<PoolIp>, DynError> {
        let row = sqlx::query(
            "SELECT ip, bridge, gateway, cidr, dns_servers, server_ip, notes FROM dhcp_pool WHERE ip = ?",
        )
        .bind(ip)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| decode_pool_row(&r)).transpose()
    }

    /// Inserts or updates pool entries. All entries are validated before anything is written.
    pub async fn upsert_pool(&self, mut entries: Vec<PoolIp>) -> Result<Vec<PoolIp>, DynError> {
        for entry in entries.iter_mut() {
            entry.validate()?;
        }
        let mut tx = self.pool.begin().await?;
        for entry in &entries {
            sqlx::query(
                "INSERT INTO dhcp_pool (ip, bridge, gateway, cidr, dns_servers, server_ip, notes, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(ip) DO UPDATE SET bridge = excluded.bridge, gateway = excluded.gateway, \
                 cidr = excluded.cidr, dns_servers = excluded.dns_servers, server_ip = excluded.server_ip, \
                 notes = excluded.notes, updated_at = excluded.updated_at",
            )
            .bind(&entry.ip)
            .bind(&entry.bridge)
            .bind(&entry.gateway)
            .bind(i64::from(entry.cidr))
            .bind(serde_json::to_string(&entry.dns_servers)?)
            .bind(entry.server_ip.clone())
            .bind(entry.notes.clone())
            .bind(now_unix())
            .execute(&mut *tx)
            .await?;
            // Keep existing leases on this IP in sync with the edited network settings.
            sqlx::query("UPDATE dhcp_leases SET gateway = ?, cidr = ?, dns_servers = ?, updated_at = ? WHERE ip = ?")
                .bind(&entry.gateway)
                .bind(i64::from(entry.cidr))
                .bind(serde_json::to_string(&entry.dns_servers)?)
                .bind(now_unix())
                .bind(&entry.ip)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        info!(count = entries.len(), "dhcp pool entries saved");
        Ok(entries)
    }

    pub async fn delete_pool_ip(&self, ip: &str) -> Result<bool, DynError> {
        let result = sqlx::query("DELETE FROM dhcp_pool WHERE ip = ?")
            .bind(ip)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ---- Simple dynamic DHCP ([[dhcp.ranges]]) ------------------------------------------

    fn range_for(&self, ip: Ipv4Addr, ifname: Option<&str>) -> Option<&DynamicRange> {
        self.ranges
            .iter()
            .find(|r| r.serves_interface(ifname) && r.contains(ip))
    }

    /// IPs that must not be handed to `mac`: static leases, live leases of other clients,
    /// and everything the admin put in the IP pool.
    async fn taken_ips(&self, mac: &str) -> Result<HashSet<Ipv4Addr>, DynError> {
        let rows = sqlx::query(
            "SELECT ip FROM dhcp_leases WHERE mac != ? AND (static_flag = 1 OR (state = 'active' AND lease_end > ?)) \
             UNION SELECT ip FROM dhcp_pool",
        )
        .bind(mac)
        .bind(now_unix())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| r.try_get::<String, _>("ip").ok())
            .filter_map(|ip| ip.parse().ok())
            .collect())
    }

    /// Picks (or keeps) a dynamic IP for a client without a static lease and stores it.
    /// `ack` = the client sent REQUEST, so the lease gets its full time; an OFFER only
    /// holds the IP for a short while. Returns the lease and its full lease time.
    async fn allocate_dynamic(
        &self,
        mac: &str,
        ifname: Option<&str>,
        existing: Option<&DhcpLease>,
        requested: Option<Ipv4Addr>,
        ack: bool,
    ) -> Result<Option<(DhcpLease, u64)>, DynError> {
        if self.ranges.is_empty() {
            return Ok(None);
        }
        let taken = self.taken_ips(mac).await?;
        let usable = |ip: Ipv4Addr| -> Option<&DynamicRange> {
            self.range_for(ip, ifname)
                .filter(|r| r.assignable(ip) && !taken.contains(&ip))
        };

        // Prefer the client's previous IP, then the one it asks for, then the first free one.
        let previous = existing.and_then(|l| l.ip.parse::<Ipv4Addr>().ok());
        let mut chosen = previous
            .and_then(|ip| usable(ip).map(|r| (ip, r)))
            .or_else(|| requested.and_then(|ip| usable(ip).map(|r| (ip, r))));
        if chosen.is_none() {
            chosen = self
                .ranges
                .iter()
                .filter(|r| r.serves_interface(ifname))
                .find_map(|r| {
                    (r.start..=r.end)
                        .map(Ipv4Addr::from)
                        .find(|ip| r.assignable(*ip) && !taken.contains(ip))
                        .map(|ip| (ip, r))
                });
        }
        let Some((ip, range)) = chosen else {
            warn!(mac, iface = ?ifname, "dhcp dynamic ranges: no free ip for client");
            return Ok(None);
        };

        let ip_s = ip.to_string();
        let same_ip = existing.filter(|l| l.ip == ip_s && l.state == "active");
        let now = now_unix();
        let lease_end = if ack {
            now + i64::try_from(range.lease_time_secs).unwrap_or(86_400)
        } else {
            same_ip
                .map(|l| l.lease_end)
                .unwrap_or(0)
                .max(now + DYNAMIC_OFFER_HOLD_SECS)
        };
        let lease_start = if ack { now } else { same_ip.map(|l| l.lease_start).unwrap_or(now) };
        let hostname = existing.and_then(|l| l.hostname.clone());

        sqlx::query(
            "INSERT INTO dhcp_leases (mac, ip, hostname, vmid, node, gateway, cidr, dns_servers, lease_start, lease_end, state, static_flag, updated_at) \
             VALUES (?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, 'active', 0, ?) \
             ON CONFLICT(mac) DO UPDATE SET \
                 ip = excluded.ip, gateway = excluded.gateway, cidr = excluded.cidr, \
                 dns_servers = excluded.dns_servers, lease_start = excluded.lease_start, \
                 lease_end = excluded.lease_end, state = 'active', updated_at = excluded.updated_at \
             WHERE dhcp_leases.static_flag = 0",
        )
        .bind(mac)
        .bind(&ip_s)
        .bind(hostname.clone())
        .bind(range.gateway.to_string())
        .bind(i64::from(range.cidr))
        .bind(serde_json::to_string(&range.dns_servers)?)
        .bind(lease_start)
        .bind(lease_end)
        .bind(now)
        .execute(&self.pool)
        .await?;

        if previous != Some(ip) {
            info!(mac, ip = %ip, iface = ?ifname, "dhcp dynamic ip allocated");
        }

        let lease = DhcpLease {
            mac: mac.to_owned(),
            ip: ip_s,
            hostname,
            vmid: None,
            node: None,
            gateway: range.gateway.to_string(),
            cidr: range.cidr,
            dns_servers: range.dns_servers.clone(),
            lease_start,
            lease_end,
            state: "active".to_owned(),
            static_lease: false,
        };
        Ok(Some((lease, range.lease_time_secs)))
    }

    pub async fn run_listener(&self, mut shutdown_rx: watch::Receiver<bool>) -> Result<(), DynError> {
        if !self.config.enabled {
            info!("dhcp listener disabled by config");
            return Ok(());
        }

        let bind: SocketAddr = self.config.bind.parse()?;
        let socket = DhcpSocket::bind(bind)?;
        info!(bind = %self.config.bind, "dhcp listener started");

        let mut buffer = [0u8; 1500];
        loop {
            select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("dhcp listener shutdown requested");
                        break;
                    }
                }
                recv = socket.recv(&mut buffer) => {
                    match recv {
                        Ok(got) => {
                            let ifname = got.ifindex.and_then(interface_name);
                            debug!(size = got.len, src = %got.src, iface = ?ifname, "dhcp packet received");
                            if let Err(err) = self
                                .handle_dhcp_packet(&socket, &buffer[..got.len], got.src, got.ifindex, ifname.as_deref())
                                .await
                            {
                                warn!(error = %err, src = %got.src, iface = ?ifname, "failed to process dhcp packet");
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
        socket: &DhcpSocket,
        packet: &[u8],
        src: SocketAddr,
        ifindex: Option<u32>,
        ifname: Option<&str>,
    ) -> Result<(), DynError> {
        let Some(request) = parse_dhcp_request(packet) else {
            return Ok(());
        };

        if request.msg_type == DHCP_RELEASE {
            let _ = self.release_lease_by_mac(&request.mac).await?;
            return Ok(());
        }

        let mut lease = self.find_lease_by_mac(&request.mac).await?;

        // Simple DHCP: clients without a static lease get an IP from [[dhcp.ranges]].
        // `dynamic` = (full lease time to advertise, server identifier of the range).
        let mut dynamic: Option<(u64, Ipv4Addr)> = None;
        let wants_dynamic = !self.ranges.is_empty()
            && matches!(request.msg_type, DHCP_DISCOVER | DHCP_REQUEST)
            && lease.as_ref().is_none_or(|l| !l.static_lease)
            && self.host_allowed(&request.mac, None);
        if wants_dynamic {
            let requested = request
                .requested_ip
                .or_else(|| (request.ciaddr != Ipv4Addr::UNSPECIFIED).then_some(request.ciaddr));
            let allocated = self
                .allocate_dynamic(
                    &request.mac,
                    ifname,
                    lease.as_ref(),
                    requested,
                    request.msg_type == DHCP_REQUEST,
                )
                .await?;
            match allocated {
                Some((new_lease, secs)) => {
                    let server = new_lease
                        .ip
                        .parse()
                        .ok()
                        .and_then(|ip| self.range_for(ip, ifname))
                        .map(|r| r.server_ip);
                    if let Some(server) = server {
                        dynamic = Some((secs, server));
                    }
                    lease = Some(new_lease);
                }
                // No free IP here: forget the stale dynamic lease so a REQUEST gets a NAK.
                None => lease = None,
            }
        }

        let lease_vmid = lease.as_ref().and_then(|l| l.vmid);
        let pool_entry = match lease.as_ref() {
            Some(l) => self.find_pool_ip(&l.ip).await?,
            None => None,
        };

        // An IP the admin put on a bridge is only handed out on that bridge.
        if let (Some(entry), Some(iface)) = (pool_entry.as_ref(), ifname) {
            if entry.bridge != iface {
                debug!(
                    mac = %request.mac,
                    ip = %entry.ip,
                    pool_bridge = %entry.bridge,
                    iface,
                    "ignoring dhcp packet: lease ip belongs to another bridge"
                );
                return Ok(());
            }
        }

        let server_ip = match dynamic
            .map(|(_, server)| server)
            .or_else(|| pool_entry.as_ref().and_then(PoolIp::server_identifier))
        {
            Some(ip) => ip,
            None => parse_ipv4(&self.config.server_ip)?,
        };

        if !self.host_allowed(&request.mac, lease_vmid) {
            let target = resolve_dhcp_target(&request, src);
            if request.msg_type == DHCP_REQUEST {
                let response = build_dhcp_nak_response(&request, server_ip);
                socket.send(&response, target, ifindex).await?;
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
                    let response = build_dhcp_nak_response(&request, server_ip);
                    let target = resolve_dhcp_target(&request, src);
                    socket.send(&response, target, ifindex).await?;
                    info!(mac = %request.mac, target = %target, "dhcp nak sent for unknown lease");
                    return Ok(());
                }
                Some(found) => {
                    if found.state != "active" {
                        let response = build_dhcp_nak_response(&request, server_ip);
                        let target = resolve_dhcp_target(&request, src);
                        socket.send(&response, target, ifindex).await?;
                        info!(mac = %request.mac, state = %found.state, target = %target, "dhcp nak sent for non-active lease");
                        return Ok(());
                    }

                    let expected = parse_ipv4(&found.ip)?;
                    let requested = request
                        .requested_ip
                        .or_else(|| (request.ciaddr != Ipv4Addr::UNSPECIFIED).then_some(request.ciaddr));
                    if let Some(req_ip) = requested {
                        if req_ip != expected {
                            let response = build_dhcp_nak_response(&request, server_ip);
                            let target = resolve_dhcp_target(&request, src);
                            socket.send(&response, target, ifindex).await?;
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

        let lease = if let Some((secs, _)) = dynamic {
            // Already stored by allocate_dynamic; the OFFER advertises the full lease time.
            let mut shown = lease;
            shown.lease_end = now_unix() + i64::try_from(secs).unwrap_or(86_400);
            shown
        } else if response_type == DHCP_ACK {
            match self.renew_lease_by_mac(&lease.mac).await? {
                Some(refreshed) => refreshed,
                other => other.unwrap_or(lease),
            }
        } else {
            lease
        };

        let response = build_dhcp_response(
            &request,
            &lease,
            response_type,
            server_ip,
            dynamic.map_or(self.config.lease_time_secs, |(secs, _)| secs),
        )?;
        let target = resolve_dhcp_target(&request, src);
        socket.send(&response, target, ifindex).await?;

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

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS dhcp_pool (                ip TEXT PRIMARY KEY,                bridge TEXT NOT NULL,                gateway TEXT NOT NULL,                cidr INTEGER NOT NULL,                dns_servers TEXT NOT NULL DEFAULT '[]',                server_ip TEXT,                notes TEXT,                updated_at INTEGER NOT NULL            )",
        )
        .execute(&self.pool)
        .await?;
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

    // Gateway outside the client's subnet (common with routed /28-/32 blocks, e.g.
    // 212.87.213.113/28 via 212.87.212.1). Keep the real mask; the DHCP reply adds
    // classless static routes (option 121) so the client reaches the gateway on-link.
    debug!(
        %ip,
        %gateway,
        cidr = requested_cidr,
        "gateway outside the subnet: will be sent as an on-link route"
    );
    Ok(requested_cidr)
}

/// True when `gateway` is not inside `ip`/`cidr` and needs an explicit on-link route.
fn gateway_needs_onlink_route(ip: Ipv4Addr, gateway: Ipv4Addr, cidr: u8) -> bool {
    let mask = subnet_mask_u32(cidr).unwrap_or(u32::MAX);
    gateway != Ipv4Addr::UNSPECIFIED && (ipv4_to_u32(ip) & mask) != (ipv4_to_u32(gateway) & mask)
}

/// RFC 3442 classless static routes: `gateway/32` directly on the link, then the default
/// route via the gateway. Clients that understand option 121 ignore option 3.
fn onlink_gateway_routes(gateway: Ipv4Addr) -> Vec<u8> {
    let mut routes = Vec::with_capacity(14);
    routes.push(32);
    routes.extend_from_slice(&gateway.octets());
    routes.extend_from_slice(&[0, 0, 0, 0]);
    routes.push(0);
    routes.extend_from_slice(&gateway.octets());
    routes
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
    configured_lease_secs: u64,
) -> Result<Vec<u8>, DynError> {
    let yiaddr = parse_ipv4(&lease.ip)?;
    let gateway = parse_ipv4(&lease.gateway)?;
    let subnet_mask = cidr_to_mask(lease.cidr);

    let now = now_unix();
    let configured = i64::try_from(configured_lease_secs).unwrap_or(86_400);
    let mut lease_time = if lease.lease_end > now {
        lease.lease_end - now
    } else {
        configured
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
    if gateway_needs_onlink_route(yiaddr, gateway, lease.cidr) {
        let routes = onlink_gateway_routes(gateway);
        push_option(&mut out, 121, &routes);
        push_option(&mut out, 249, &routes);
    }

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

fn decode_pool_row(row: &sqlx::sqlite::SqliteRow) -> Result<PoolIp, DynError> {
    let cidr_i64: i64 = row.try_get("cidr")?;
    let dns_raw: String = row.try_get("dns_servers")?;
    Ok(PoolIp {
        ip: row.try_get("ip")?,
        bridge: row.try_get("bridge")?,
        gateway: row.try_get("gateway")?,
        cidr: u8::try_from(cidr_i64).map_err(|_| "invalid cidr in pool row")?,
        dns_servers: serde_json::from_str(&dns_raw).unwrap_or_default(),
        server_ip: row.try_get("server_ip")?,
        notes: row.try_get("notes")?,
    })
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

    fn pool_ip(ip: &str, bridge: &str) -> PoolIp {
        PoolIp {
            ip: ip.into(),
            bridge: bridge.into(),
            gateway: "10.0.0.1".into(),
            cidr: 24,
            dns_servers: vec!["1.1.1.1".into()],
            server_ip: None,
            notes: None,
        }
    }

    #[test]
    fn expand_ip_list_handles_ranges_and_lists() {
        let ips = expand_ip_list("10.0.0.40-42, 10.0.0.50\n10.0.0.60-10.0.0.61").unwrap();
        assert_eq!(
            ips,
            vec!["10.0.0.40", "10.0.0.41", "10.0.0.42", "10.0.0.50", "10.0.0.60", "10.0.0.61"]
        );
        assert!(expand_ip_list("10.0.0.5-1").is_err());
        assert!(expand_ip_list("10.0.0.0-10.0.255.255").is_err());
        assert!(expand_ip_list("nope").is_err());
    }

    #[test]
    fn pool_ip_validation() {
        let mut ok = pool_ip("10.0.0.40", "vmbr1");
        ok.validate().unwrap();
        assert_eq!(ok.server_identifier(), Some(Ipv4Addr::new(10, 0, 0, 1)));

        assert!(pool_ip("10.0.0.1", "vmbr1").validate().is_err(), "gateway itself");
        // Gateway in another subnet is allowed (sent as an on-link route).
        assert!(pool_ip("10.0.5.4", "vmbr1").validate().is_ok(), "routed gateway");
        assert!(pool_ip("10.0.0.4", "vmbr1; rm -rf").validate().is_err(), "bad bridge");

        let mut with_server = pool_ip("10.0.0.41", "vmbr1");
        with_server.server_ip = Some(" 10.0.0.2 ".into());
        with_server.validate().unwrap();
        assert_eq!(with_server.server_identifier(), Some(Ipv4Addr::new(10, 0, 0, 2)));
    }

    async fn temp_service() -> (DhcpService, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "ft-dhcp-{}-{}.sqlite3",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let cfg = DhcpConfig {
            database_path: path.to_string_lossy().to_string(),
            ..DhcpConfig::default()
        };
        (DhcpService::new(cfg).await.unwrap(), path)
    }

    #[tokio::test]
    async fn pool_and_multi_nic_leases() {
        let (svc, path) = temp_service().await;
        svc.upsert_pool(vec![pool_ip("10.0.0.40", "vmbr1"), pool_ip("10.0.0.41", "vmbr1")])
            .await
            .unwrap();
        let entry = svc.find_pool_ip("10.0.0.40").await.unwrap().unwrap();
        assert_eq!(entry.bridge, "vmbr1");
        assert_eq!(entry.server_identifier(), Some(Ipv4Addr::new(10, 0, 0, 1)));

        let lease = |mac: &str, ip: &str, replace: bool| AssignLeaseInput {
            mac: mac.into(),
            ip: ip.into(),
            hostname: None,
            vmid: Some(100),
            node: None,
            gateway: "10.0.0.1".into(),
            cidr: 24,
            dns_servers: vec![],
            lease_time_secs: None,
            replace_vmid_leases: replace,
        };

        // Admin UI path: two NICs of the same VM keep separate leases.
        svc.assign_static_lease(lease("aa:aa:aa:aa:aa:01", "10.0.0.40", false)).await.unwrap();
        svc.assign_static_lease(lease("aa:aa:aa:aa:aa:02", "10.0.0.41", false)).await.unwrap();
        assert_eq!(svc.list_leases().await.unwrap().len(), 2);

        // Conflict detection sees the other MAC holding the IP.
        let holder = svc.lease_holding_ip("10.0.0.40", "aa:aa:aa:aa:aa:02").await.unwrap();
        assert_eq!(holder.unwrap().mac, "aa:aa:aa:aa:aa:01");
        assert!(svc.lease_holding_ip("10.0.0.40", "aa:aa:aa:aa:aa:01").await.unwrap().is_none());

        // Editing the pool entry syncs the lease's network settings.
        let mut edited = pool_ip("10.0.0.40", "vmbr1");
        edited.dns_servers = vec!["9.9.9.9".into()];
        svc.upsert_pool(vec![edited]).await.unwrap();
        let l = svc.find_lease_by_mac("aa:aa:aa:aa:aa:01").await.unwrap().unwrap();
        assert_eq!(l.dns_servers, vec!["9.9.9.9"]);

        // FeatherPanel path keeps its one-lease-per-VM behaviour.
        svc.assign_static_lease(lease("aa:aa:aa:aa:aa:03", "10.0.0.42", true)).await.unwrap();
        assert_eq!(svc.list_leases().await.unwrap().len(), 1);

        assert!(svc.remove_lease_by_mac("AA-AA-AA-AA-AA-03").await.unwrap());
        assert!(svc.delete_pool_ip("10.0.0.41").await.unwrap());
        svc.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn dynamic_ranges_hand_out_free_ips() {
        let path = std::env::temp_dir().join(format!(
            "ft-dhcp-dyn-{}-{}.sqlite3",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let cfg = DhcpConfig {
            database_path: path.to_string_lossy().to_string(),
            ranges: vec![
                DhcpRange {
                    interface: "vmbr1".into(),
                    start: "10.0.0.1".into(),
                    end: "10.0.0.4".into(),
                    gateway: "10.0.0.1".into(),
                    cidr: 24,
                    dns_servers: vec!["1.1.1.1".into()],
                    server_ip: None,
                    lease_time_secs: Some(600),
                },
                // Invalid (start and end in different /24s): skipped at startup.
                DhcpRange {
                    interface: String::new(),
                    start: "192.168.5.10".into(),
                    end: "192.168.6.20".into(),
                    gateway: "10.0.0.1".into(),
                    cidr: 24,
                    dns_servers: vec![],
                    server_ip: None,
                    lease_time_secs: None,
                },
            ],
            ..DhcpConfig::default()
        };
        let svc = DhcpService::new(cfg).await.unwrap();
        assert_eq!(svc.ranges.len(), 1);
        let a = "aa:00:00:00:00:01";
        let b = "aa:00:00:00:00:02";

        // .1 is the gateway; the admin pool reserves .2 -> first client gets .3.
        svc.upsert_pool(vec![pool_ip("10.0.0.2", "vmbr1")]).await.unwrap();
        let (offer, secs) = svc.allocate_dynamic(a, Some("vmbr1"), None, None, false).await.unwrap().unwrap();
        assert_eq!((offer.ip.as_str(), secs), ("10.0.0.3", 600));
        assert!(!offer.static_lease);

        // Wrong interface: nothing served.
        assert!(svc.allocate_dynamic(b, Some("vmbr0"), None, None, false).await.unwrap().is_none());

        // Second client cannot take .3 even if it asks for it.
        let req = Some(Ipv4Addr::new(10, 0, 0, 3));
        let (other, _) = svc.allocate_dynamic(b, Some("vmbr1"), None, req, true).await.unwrap().unwrap();
        assert_eq!(other.ip, "10.0.0.4");

        // First client's REQUEST keeps its IP and gets the full lease.
        let existing = svc.find_lease_by_mac(a).await.unwrap();
        let (acked, _) = svc.allocate_dynamic(a, Some("vmbr1"), existing.as_ref(), req, true).await.unwrap().unwrap();
        assert_eq!(acked.ip, "10.0.0.3");
        assert!(acked.lease_end >= now_unix() + 590);

        // Range full.
        assert!(svc.allocate_dynamic("aa:00:00:00:00:03", Some("vmbr1"), None, None, false).await.unwrap().is_none());

        // A released lease frees its IP for someone else.
        svc.release_lease_by_mac(b).await.unwrap();
        let (c, _) = svc.allocate_dynamic("aa:00:00:00:00:03", Some("vmbr1"), None, None, false).await.unwrap().unwrap();
        assert_eq!(c.ip, "10.0.0.4");

        svc.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

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
    fn build_dhcp_response_uses_configured_lease_when_db_expired() {
        let packet = base_packet_with_options(&[53, 1, DHCP_DISCOVER, 255]);
        let parsed = parse_dhcp_request(&packet).expect("packet should parse");

        let lease = DhcpLease {
            mac: "aa:bb:cc:dd:ee:ff".to_owned(),
            ip: "10.0.0.10".to_owned(),
            hostname: None,
            vmid: None,
            node: None,
            gateway: "10.0.0.1".to_owned(),
            cidr: 24,
            dns_servers: vec![],
            lease_start: 0,
            lease_end: 1,
            state: "active".to_owned(),
            static_lease: true,
        };

        let response = build_dhcp_response(
            &parsed,
            &lease,
            DHCP_OFFER,
            Ipv4Addr::new(10, 0, 0, 1),
            7200,
        )
        .expect("response should build");

        let lease_secs = 7200u32.to_be_bytes();
        assert!(response.windows(6).any(|w| w == [51, 4, lease_secs[0], lease_secs[1], lease_secs[2], lease_secs[3]]));
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

        let response = build_dhcp_response(
            &parsed,
            &lease,
            DHCP_OFFER,
            Ipv4Addr::new(193, 34, 77, 2),
            86_400,
        )
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
    fn effective_cidr_keeps_mask_for_routed_gateway() {
        // Routed /28 with the provider gateway in another /23.
        let ip = Ipv4Addr::new(212, 87, 213, 113);
        let gw = Ipv4Addr::new(212, 87, 212, 1);
        assert_eq!(effective_cidr_for_gateway_and_ip(ip, gw, 28).unwrap(), 28);
        assert!(gateway_needs_onlink_route(ip, gw, 28));
        assert!(!gateway_needs_onlink_route(Ipv4Addr::new(10, 0, 0, 5), Ipv4Addr::new(10, 0, 0, 1), 24));
    }

    #[test]
    fn dhcp_response_adds_onlink_route_for_routed_gateway() {
        let request = parse_dhcp_request(&base_packet_with_options(&[53, 1, 1, 255])).unwrap();
        let lease = DhcpLease {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            ip: "212.87.213.113".into(),
            hostname: None,
            vmid: None,
            node: None,
            gateway: "212.87.212.1".into(),
            cidr: 28,
            dns_servers: vec![],
            lease_start: 0,
            lease_end: 0,
            state: "active".into(),
            static_lease: true,
        };
        let out = build_dhcp_response(&request, &lease, DHCP_OFFER, Ipv4Addr::new(212, 87, 212, 1), 3600).unwrap();
        let expected = [121u8, 14, 32, 212, 87, 212, 1, 0, 0, 0, 0, 0, 212, 87, 212, 1];
        assert!(out.windows(expected.len()).any(|w| w == expected));
        // Mask stays /28.
        assert!(out.windows(6).any(|w| w == [1, 4, 255, 255, 255, 240]));
    }
}
