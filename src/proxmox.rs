use std::error::Error;
use std::io;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use utoipa::ToSchema;

use crate::config::ProxmoxConfig;

type DynError = Box<dyn Error + Send + Sync>;

pub struct ProxmoxClient {
    pvesh_bin: String,
}

impl ProxmoxClient {
    pub fn new(config: &ProxmoxConfig) -> Self {
        Self {
            pvesh_bin: config.pvesh_bin.clone(),
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

    async fn set_lxc_password_on_node(&self, node: &str, vmid: u32, password: &str) -> Result<(), DynError> {
        let path = format!("/nodes/{node}/lxc/{vmid}/config");
        let output = Command::new(&self.pvesh_bin)
            .arg("set")
            .arg(&path)
            .arg("-password")
            .arg(password)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let err = io::Error::other(format!("pvesh set {path} failed: {stderr}"));
            return Err(Box::new(err));
        }

        Ok(())
    }

    pub async fn execute(&self, method: &str, path: &str, params: &[(String, String)]) -> Result<Value, DynError> {
        let mut cmd = Command::new(&self.pvesh_bin);
        cmd.arg(method).arg(path);

        for (key, value) in params {
            cmd.arg(format!("-{key}")).arg(value);
        }

        if method == "get" {
            cmd.arg("--output-format").arg("json");
        }

        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let err = io::Error::other(format!("pvesh {method} {path} failed: {stderr}"));
            return Err(Box::new(err));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if stdout.is_empty() {
            return Ok(json!({ "status": "ok" }));
        }

        if let Ok(parsed) = serde_json::from_str::<Value>(&stdout) {
            return Ok(parsed);
        }

        Ok(json!({ "output": stdout }))
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
