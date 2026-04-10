use std::error::Error;
use std::io;

use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use utoipa::ToSchema;

use crate::config::ProxmoxConfig;

type DynError = Box<dyn Error + Send + Sync>;

pub struct ProxmoxClient {
    pvesh_bin: String,
    pct_bin: String,
}

impl ProxmoxClient {
    pub fn new(config: &ProxmoxConfig) -> Self {
        Self {
            pvesh_bin: config.pvesh_bin.clone(),
            pct_bin: config.pct_bin.clone(),
        }
    }

    pub async fn version(&self) -> Result<ProxmoxVersion, DynError> {
        let output = self.get_json("/version").await?;
        let parsed = serde_json::from_value(output)?;
        Ok(parsed)
    }

    pub async fn nodes(&self) -> Result<Vec<ProxmoxNode>, DynError> {
        let output = self.get_json("/nodes").await?;
        let parsed = serde_json::from_value(output)?;
        Ok(parsed)
    }

    pub async fn qemu_list(&self) -> Result<Vec<QemuVM>, DynError> {
        let nodes = self.nodes().await?;
        let mut vms = Vec::new();

        for node in nodes {
            let path = format!("/nodes/{}/qemu", node.node);
            if let Ok(output) = self.get_json(&path).await {
                if let Ok(list) = serde_json::from_value::<Vec<Value>>(output) {
                    for item in list {
                        if let Ok(mut vm) = serde_json::from_value::<QemuVM>(item) {
                            vm.node = node.node.clone();
                            vms.push(vm);
                        }
                    }
                }
            }
        }

        Ok(vms)
    }

    pub async fn lxc_list(&self) -> Result<Vec<LxcContainer>, DynError> {
        let nodes = self.nodes().await?;
        let mut containers = Vec::new();

        for node in nodes {
            let path = format!("/nodes/{}/lxc", node.node);
            if let Ok(output) = self.get_json(&path).await {
                if let Ok(list) = serde_json::from_value::<Vec<Value>>(output) {
                    for item in list {
                        if let Ok(mut container) = serde_json::from_value::<LxcContainer>(item) {
                            container.node = node.node.clone();
                            containers.push(container);
                        }
                    }
                }
            }
        }

        Ok(containers)
    }

    pub async fn set_lxc_root_password(&self, vmid: u32, password: &str) -> Result<String, DynError> {
        let container = self
            .lxc_list()
            .await?
            .into_iter()
            .find(|ct| ct.vmid == vmid)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("lxc container {vmid} not found")))?;

        self.set_lxc_password_on_node(&container.node, vmid, password)
            .await?;

        Ok(container.node)
    }

    pub async fn resolve_qemu_identity(&self, vmid: u32) -> Result<QemuIdentity, DynError> {
        let vm = self
            .qemu_list()
            .await?
            .into_iter()
            .find(|vm| vm.vmid == vmid)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("qemu vm {vmid} not found")))?;

        let config_path = format!("/nodes/{}/qemu/{}/config", vm.node, vmid);
        let config = self.get_json(&config_path).await?;

        let mac = extract_qemu_mac(&config)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no network mac found for qemu vm {vmid}")))?;

        Ok(QemuIdentity {
            vmid,
            node: vm.node,
            mac,
        })
    }

    async fn set_lxc_password_on_node(&self, node: &str, vmid: u32, password: &str) -> Result<(), DynError> {
        let status_output = Command::new(&self.pct_bin)
            .arg("status")
            .arg(vmid.to_string())
            .output()
            .await?;

        if !status_output.status.success() {
            let stderr = String::from_utf8_lossy(&status_output.stderr).trim().to_owned();
            let err = io::Error::other(format!("pct status {vmid} failed on node {node}: {stderr}"));
            return Err(Box::new(err));
        }

        let status_stdout = String::from_utf8_lossy(&status_output.stdout);
        if !status_stdout.contains("status: running") {
            let err = io::Error::other(format!(
                "container {vmid} on node {node} must be running to change root password"
            ));
            return Err(Box::new(err));
        }

        let mut child = Command::new(&self.pct_bin)
            .arg("exec")
            .arg(vmid.to_string())
            .arg("--")
            .arg("chpasswd")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("failed to open pct exec stdin"))?;
        let input = format!("root:{password}\n");
        stdin.write_all(input.as_bytes()).await?;
        stdin.shutdown().await?;

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let err = io::Error::other(format!("pct exec {vmid} -- chpasswd failed on node {node}: {stderr}"));
            return Err(Box::new(err));
        }

        Ok(())
    }

    async fn get_json(&self, path: &str) -> Result<Value, DynError> {
        let output = Command::new(&self.pvesh_bin)
            .arg("get")
            .arg(path)
            .arg("--output-format")
            .arg("json")
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let err = io::Error::other(format!("pvesh get {path} failed: {stderr}"));
            return Err(Box::new(err));
        }

        let parsed: Value = serde_json::from_slice(&output.stdout)?;
        Ok(parsed)
    }
}

fn extract_qemu_mac(config: &Value) -> Option<String> {
    let map = config.as_object()?;

    for idx in 0..=9 {
        let key = format!("net{idx}");
        let Some(raw) = map.get(&key) else {
            continue;
        };
        let Some(value) = raw.as_str() else {
            continue;
        };

        let Some(first) = value.split(',').next() else {
            continue;
        };
        let Some(mac_raw) = first.split('=').nth(1) else {
            continue;
        };
        if let Some(mac) = normalize_mac_string(mac_raw.trim()) {
            return Some(mac);
        }
    }

    None
}

fn normalize_mac_string(value: &str) -> Option<String> {
    let stripped: String = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();

    if stripped.len() != 12 {
        return None;
    }

    Some(format!(
        "{}:{}:{}:{}:{}:{}",
        &stripped[0..2],
        &stripped[2..4],
        &stripped[4..6],
        &stripped[6..8],
        &stripped[8..10],
        &stripped[10..12],
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProxmoxVersion {
    pub version: String,
    pub release: String,
    pub repoid: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProxmoxNode {
    pub node: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct QemuVM {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: u32,
    pub vmid: u32,
    pub name: String,
    #[serde(default)]
    pub node: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub maxmem: Option<u64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LxcContainer {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: u32,
    pub vmid: u32,
    pub name: String,
    #[serde(default)]
    pub node: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub maxmem: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct QemuIdentity {
    pub vmid: u32,
    pub node: String,
    pub mac: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_mac_string_handles_mixed_formats() {
        let mac = normalize_mac_string("AA:BB:CC:DD:EE:FF").expect("valid mac");
        assert_eq!(mac, "aa:bb:cc:dd:ee:ff");

        let mac2 = normalize_mac_string("aabbccddeeff").expect("valid compact mac");
        assert_eq!(mac2, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn normalize_mac_string_rejects_invalid() {
        assert!(normalize_mac_string("zz:11:22:33:44:55").is_none());
        assert!(normalize_mac_string("aa:bb:cc:dd:ee").is_none());
    }

    #[test]
    fn extract_qemu_mac_prefers_first_available_net() {
        let config = serde_json::json!({
            "net0": "virtio=AA:BB:CC:DD:EE:FF,bridge=vmbr0",
            "net1": "virtio=11:22:33:44:55:66,bridge=vmbr1"
        });

        let mac = extract_qemu_mac(&config).expect("mac should be parsed");
        assert_eq!(mac, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn extract_qemu_mac_returns_none_without_nets() {
        let config = serde_json::json!({ "name": "vm-no-net" });
        assert!(extract_qemu_mac(&config).is_none());
    }
}
