use std::error::Error;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub proxmox: ProxmoxConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub dhcp: DhcpConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            proxmox: ProxmoxConfig::default(),
            api: ApiConfig::default(),
            auth: AuthConfig::default(),
            dhcp: DhcpConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_bootstrap(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }
            let default = Self::default();
            let serialized = toml::to_string_pretty(&default)?;
            fs::write(path, serialized)?;
            return Ok(default);
        }

        let contents = fs::read_to_string(path)?;
        let parsed: Self = toml::from_str(&contents)?;
        Ok(parsed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_daemon_name")]
    pub name: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            name: default_daemon_name(),
            poll_interval_secs: default_poll_interval(),
            log_level: default_log_level(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxmoxConfig {
    #[serde(default = "default_pvesh_bin")]
    pub pvesh_bin: String,
    #[serde(default = "default_pct_bin")]
    pub pct_bin: String,
}

impl Default for ProxmoxConfig {
    fn default() -> Self {
        Self {
            pvesh_bin: default_pvesh_bin(),
            pct_bin: default_pct_bin(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_api_bind")]
    pub bind: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: default_api_bind(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_api_token")]
    pub api_token: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            api_token: default_api_token(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhcpConfig {
    #[serde(default = "default_dhcp_enabled")]
    pub enabled: bool,
    #[serde(default = "default_dhcp_bind")]
    pub bind: String,
    #[serde(default = "default_dhcp_server_ip")]
    pub server_ip: String,
    #[serde(default = "default_dhcp_lease_time_secs")]
    pub lease_time_secs: u64,
    #[serde(default = "default_dhcp_database_path")]
    pub database_path: String,
    #[serde(default = "default_dhcp_firewall_mode")]
    pub firewall_mode: String,
    #[serde(default)]
    pub firewall_allow_macs: Vec<String>,
    #[serde(default)]
    pub firewall_deny_macs: Vec<String>,
    #[serde(default)]
    pub firewall_allow_vmids: Vec<u32>,
    #[serde(default)]
    pub firewall_deny_vmids: Vec<u32>,
}

impl Default for DhcpConfig {
    fn default() -> Self {
        Self {
            enabled: default_dhcp_enabled(),
            bind: default_dhcp_bind(),
            server_ip: default_dhcp_server_ip(),
            lease_time_secs: default_dhcp_lease_time_secs(),
            database_path: default_dhcp_database_path(),
            firewall_mode: default_dhcp_firewall_mode(),
            firewall_allow_macs: Vec::new(),
            firewall_deny_macs: Vec::new(),
            firewall_allow_vmids: Vec::new(),
            firewall_deny_vmids: Vec::new(),
        }
    }
}

fn default_daemon_name() -> String {
    "feathertail-proxmox".to_owned()
}

fn default_poll_interval() -> u64 {
    15
}

fn default_log_level() -> String {
    "info".to_owned()
}

fn default_pvesh_bin() -> String {
    "pvesh".to_owned()
}

fn default_pct_bin() -> String {
    "pct".to_owned()
}

fn default_api_bind() -> String {
    "127.0.0.1:8686".to_owned()
}

fn default_api_token() -> String {
    "change-me".to_owned()
}

fn default_dhcp_enabled() -> bool {
    false
}

fn default_dhcp_bind() -> String {
    "0.0.0.0:67".to_owned()
}

fn default_dhcp_server_ip() -> String {
    "10.0.0.1".to_owned()
}

fn default_dhcp_lease_time_secs() -> u64 {
    86_400
}

fn default_dhcp_database_path() -> String {
    "/var/lib/feathertail/dhcp.sqlite3".to_owned()
}

fn default_dhcp_firewall_mode() -> String {
    "off".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("feathertail-{name}-{nanos}.toml"))
    }

    #[test]
    fn default_config_has_expected_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.proxmox.pvesh_bin, "pvesh");
        assert_eq!(cfg.proxmox.pct_bin, "pct");
        assert_eq!(cfg.dhcp.firewall_mode, "off");
        assert_eq!(cfg.dhcp.lease_time_secs, 86_400);
    }

    #[test]
    fn load_or_bootstrap_creates_file_and_parent() {
        let path = unique_test_path("bootstrap");
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if path.exists() {
            let _ = fs::remove_file(&path);
        }

        let loaded = AppConfig::load_or_bootstrap(&path).expect("bootstrap should succeed");
        assert!(path.exists());
        assert_eq!(loaded.daemon.name, "feathertail-proxmox");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_or_bootstrap_reads_existing_custom_values() {
        let path = unique_test_path("custom");
        let contents = r#"
[daemon]
name = "custom-daemon"
poll_interval_secs = 30
log_level = "warn"

[proxmox]
pvesh_bin = "/usr/bin/pvesh"
pct_bin = "/usr/sbin/pct"

[api]
bind = "127.0.0.1:9999"

[auth]
api_token = "token"

[dhcp]
enabled = true
bind = "0.0.0.0:67"
server_ip = "193.34.77.2"
lease_time_secs = 1200
database_path = "/tmp/test.sqlite"
firewall_mode = "allowlist"
firewall_allow_macs = ["aa:bb:cc:dd:ee:ff"]
firewall_allow_vmids = [100]
"#;

        fs::write(&path, contents).expect("fixture write should succeed");
        let loaded = AppConfig::load_or_bootstrap(&path).expect("parse should succeed");

        assert_eq!(loaded.daemon.name, "custom-daemon");
        assert_eq!(loaded.proxmox.pct_bin, "/usr/sbin/pct");
        assert_eq!(loaded.dhcp.firewall_mode, "allowlist");
        assert_eq!(loaded.dhcp.firewall_allow_vmids, vec![100]);

        let _ = fs::remove_file(&path);
    }
}
