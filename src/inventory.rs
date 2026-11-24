//! Read-only view of guests and host bridges for the admin UI.
//!
//! Guest NICs come straight from the Proxmox config files under /etc/pve (one file read per
//! guest instead of one `pvesh` call per guest). Nothing here ever changes host or guest config.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use tokio::process::Command;
use utoipa::ToSchema;

const PVE_NODES_DIR: &str = "/etc/pve/nodes";

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GuestNic {
    /// net0, net1, ...
    pub key: String,
    pub mac: String,
    pub bridge: Option<String>,
    pub model: Option<String>,
    pub vlan: Option<String>,
    /// LXC only: the `ip=` setting (dhcp / static / manual).
    pub ip_config: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Guest {
    pub vmid: u32,
    /// "qemu" or "lxc"
    pub kind: String,
    pub name: String,
    pub node: String,
    pub status: Option<String>,
    pub template: bool,
    pub nics: Vec<GuestNic>,
    /// Proxmox tags, in config order.
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HostBridge {
    pub name: String,
    pub state: Option<String>,
    pub addresses: Vec<String>,
}

/// Lists all guests visible in /etc/pve (cluster-wide), enriched with live status when available.
pub async fn list_guests(pvesh_bin: &str) -> Vec<Guest> {
    let statuses = cluster_statuses(pvesh_bin).await;
    let mut guests = tokio::task::spawn_blocking(read_guest_configs)
        .await
        .unwrap_or_default();
    for guest in &mut guests {
        if let Some(status) = statuses.get(&guest.vmid) {
            guest.status = Some(status.clone());
        }
    }
    guests.sort_by_key(|g| g.vmid);
    guests
}

pub async fn find_guest(pvesh_bin: &str, vmid: u32) -> Option<Guest> {
    list_guests(pvesh_bin).await.into_iter().find(|g| g.vmid == vmid)
}

async fn cluster_statuses(pvesh_bin: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let Ok(output) = Command::new(pvesh_bin)
        .args(["get", "/cluster/resources", "--type", "vm", "--output-format", "json"])
        .output()
        .await
    else {
        return map;
    };
    if !output.status.success() {
        return map;
    }
    let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) else {
        return map;
    };
    for item in items {
        let vmid = item.get("vmid").and_then(|v| v.as_u64());
        let status = item.get("status").and_then(|v| v.as_str());
        if let (Some(vmid), Some(status)) = (vmid, status) {
            if let Ok(vmid) = u32::try_from(vmid) {
                map.insert(vmid, status.to_owned());
            }
        }
    }
    map
}

fn read_guest_configs() -> Vec<Guest> {
    let mut out = Vec::new();
    // Override only used for local development (`feathertail --dev`, see dev/pve).
    let dir = std::env::var("FEATHERTAIL_PVE_NODES_DIR").unwrap_or_else(|_| PVE_NODES_DIR.to_owned());
    let Ok(nodes) = std::fs::read_dir(&dir) else {
        return out;
    };
    for node in nodes.flatten() {
        let node_name = node.file_name().to_string_lossy().to_string();
        for (sub, kind) in [("qemu-server", "qemu"), ("lxc", "lxc")] {
            let dir = node.path().join(sub);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(guest) = read_guest_file(&path, kind, &node_name) {
                    out.push(guest);
                }
            }
        }
    }
    out
}

fn read_guest_file(path: &Path, kind: &str, node: &str) -> Option<Guest> {
    if path.extension().and_then(|e| e.to_str()) != Some("conf") {
        return None;
    }
    let vmid: u32 = path.file_stem()?.to_str()?.parse().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    Some(parse_guest_config(vmid, kind, node, &contents))
}

/// Parses the current (non-snapshot) section of a qemu-server / lxc config file.
pub fn parse_guest_config(vmid: u32, kind: &str, node: &str, contents: &str) -> Guest {
    let mut name = None;
    let mut template = false;
    let mut nics = Vec::new();
    let mut tags = Vec::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break; // snapshot / pending sections follow
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim();
        let value = value.trim();
        match key {
            "name" | "hostname" => name = Some(value.to_owned()),
            "template" => template = value == "1",
            "tags" => tags = parse_tags(value),
            k if k.starts_with("net") && k[3..].chars().all(|c| c.is_ascii_digit()) && k.len() > 3 => {
                if let Some(nic) = parse_nic(k, kind, value) {
                    nics.push(nic);
                }
            }
            _ => {}
        }
    }

    nics.sort_by_key(|n| n.key[3..].parse::<u32>().unwrap_or(u32::MAX));

    Guest {
        vmid,
        kind: kind.to_owned(),
        name: name.unwrap_or_else(|| format!("{kind}-{vmid}")),
        node: node.to_owned(),
        status: None,
        template,
        nics,
        tags,
    }
}

/// Proxmox accepts `;`, `,` and spaces as tag separators.
pub fn parse_tags(value: &str) -> Vec<String> {
    value
        .split([';', ',', ' '])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_nic(key: &str, kind: &str, value: &str) -> Option<GuestNic> {
    let mut mac = None;
    let mut model = None;
    let mut bridge = None;
    let mut vlan = None;
    let mut ip_config = None;

    for part in value.split(',') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "bridge" => bridge = Some(v.to_owned()),
            "tag" => vlan = Some(v.to_owned()),
            "hwaddr" if kind == "lxc" => mac = Some(v.to_owned()),
            "ip" if kind == "lxc" => ip_config = Some(v.to_owned()),
            "virtio" | "e1000" | "e1000e" | "rtl8139" | "vmxnet3" | "ne2k_pci" | "pcnet"
            | "i82551" | "i82557b" | "i82559er" | "ne2k_isa"
                if kind == "qemu" =>
            {
                model = Some(k.to_owned());
                mac = Some(v.to_owned());
            }
            "macaddr" if mac.is_none() => mac = Some(v.to_owned()),
            _ => {}
        }
    }

    let mac = crate::dhcp::normalize_mac(&mac?).ok()?;
    Some(GuestNic {
        key: key.to_owned(),
        mac,
        bridge,
        model,
        vlan,
        ip_config,
    })
}

/// Bridges that exist on this host right now. Used only as suggestions in the UI.
pub async fn list_host_bridges() -> Vec<HostBridge> {
    let mut bridges: Vec<HostBridge> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return bridges;
    };
    for entry in entries.flatten() {
        if !entry.path().join("bridge").exists() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let state = std::fs::read_to_string(entry.path().join("operstate"))
            .ok()
            .map(|s| s.trim().to_owned());
        bridges.push(HostBridge {
            name,
            state,
            addresses: Vec::new(),
        });
    }

    if let Ok(output) = Command::new("ip").args(["-j", "-4", "addr", "show"]).output().await {
        if let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) {
            for item in items {
                let Some(ifname) = item.get("ifname").and_then(|v| v.as_str()) else { continue };
                let Some(bridge) = bridges.iter_mut().find(|b| b.name == ifname) else { continue };
                for addr in item
                    .get("addr_info")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    let local = addr.get("local").and_then(|v| v.as_str());
                    let prefix = addr.get("prefixlen").and_then(|v| v.as_u64());
                    if let (Some(local), Some(prefix)) = (local, prefix) {
                        bridge.addresses.push(format!("{local}/{prefix}"));
                    }
                }
            }
        }
    }

    bridges.sort_by(|a, b| a.name.cmp(&b.name));
    bridges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qemu_nics_and_skips_snapshots() {
        let cfg = "name: web01\nnet0: virtio=BC:24:11:AA:BB:CC,bridge=vmbr0,firewall=1\nnet1: e1000=bc:24:11:00:00:01,bridge=vmbr1,tag=20\n\n[snap1]\nnet2: virtio=BC:24:11:FF:FF:FF,bridge=vmbr9\n";
        let g = parse_guest_config(101, "qemu", "pve1", cfg);
        assert_eq!(g.name, "web01");
        assert_eq!(g.nics.len(), 2);
        assert_eq!(g.nics[0].mac, "bc:24:11:aa:bb:cc");
        assert_eq!(g.nics[0].bridge.as_deref(), Some("vmbr0"));
        assert_eq!(g.nics[1].vlan.as_deref(), Some("20"));
        assert_eq!(g.nics[1].model.as_deref(), Some("e1000"));
    }

    #[test]
    fn parses_tags_with_any_separator() {
        let guest = parse_guest_config(100, "qemu", "pve1", "name: a
tags: prod;10.0.0.5, web x
");
        assert_eq!(guest.tags, vec!["prod", "10.0.0.5", "web", "x"]);
    }

    #[test]
    fn parses_lxc_nics() {
        let cfg = "hostname: ct1\nnet0: name=eth0,bridge=vmbr1,hwaddr=BC:24:11:12:34:56,ip=dhcp,type=veth\n";
        let g = parse_guest_config(200, "lxc", "pve1", cfg);
        assert_eq!(g.name, "ct1");
        assert_eq!(g.nics[0].mac, "bc:24:11:12:34:56");
        assert_eq!(g.nics[0].ip_config.as_deref(), Some("dhcp"));
    }
}
