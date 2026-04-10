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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            proxmox: ProxmoxConfig::default(),
            api: ApiConfig::default(),
            auth: AuthConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_bootstrap(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
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
}

impl Default for ProxmoxConfig {
    fn default() -> Self {
        Self {
            pvesh_bin: default_pvesh_bin(),
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

fn default_api_bind() -> String {
    "127.0.0.1:8686".to_owned()
}

fn default_api_token() -> String {
    "change-me".to_owned()
}
