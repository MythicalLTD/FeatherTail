# FeatherTail

FeatherTail is a Rust daemon for Proxmox hosts that provides:

- DHCP lease management API tied to VM identity
- DHCP packet responses (OFFER/ACK/NAK/RELEASE handling)
- Proxmox VM and container API helpers
- noVNC helper asset installation on startup

FeatherTail is designed to run only on Proxmox hosts.

## Features

- Proxmox host guard: exits if host is not Proxmox
- Bundled noVNC assets copied to /usr/share/novnc-pve
- DHCP leases assigned by VMID with per-lease network settings
- DHCP firewall modes: off, blocklist, allowlist
- API docs at /docs and OpenAPI at /openapi.json
- LXC root password endpoint

## Requirements

- Linux host
- Proxmox VE host
- root privileges (needed for binding UDP 67, system paths, service install)
- systemd (for service-install command)

## Quick Start (Binary Install)

1. Download binary from releases:

```bash
wget -O feathertail https://github.com/mythicalltd/feathertail/releases/latest/download/feathertail-linux-amd64
chmod +x feathertail
```

2. Install as service:

```bash
sudo ./feathertail service-install
```

What this does:

- copies binary to /usr/local/bin/feathertail
- creates config at /etc/feathertail/feathertail.toml (or copies your current one)
- creates systemd unit at /etc/systemd/system/feathertail.service
- runs systemctl daemon-reload
- runs systemctl enable --now feathertail.service

3. Check status:

```bash
systemctl status feathertail
journalctl -u feathertail -f
```

## Development Setup

```bash
make install-builtkit
cargo build
cargo test
cargo run -- --config ./feathertail.toml
```


## Configuration

Example config:

```toml
[daemon]
name = "feathertail-proxmox"
poll_interval_secs = 15
log_level = "info-yapless"

[proxmox]
pvesh_bin = "pvesh"
pct_bin = "pct"

[api]
bind = "0.0.0.0:8686"

[auth]
api_token = "change-me"

[dhcp]
enabled = true
bind = "0.0.0.0:67"
server_ip = "10.0.0.1"
lease_time_secs = 86400
database_path = "/var/lib/feathertail/dhcp.sqlite3"
firewall_mode = "off"
firewall_allow_macs = []
firewall_deny_macs = []
firewall_allow_vmids = []
firewall_deny_vmids = []
```

## API Overview

Base path: /api/v1

- GET /dhcp/leases
- POST /dhcp/leases
- DELETE /dhcp/leases/vm/{vmid}
- GET /servers
- GET /containers
- POST /containers/{vmid}/root-password
- GET /proxmox/version
- GET /proxmox/nodes

## DHCP Lease Example

```bash
curl -X POST http://127.0.0.1:8686/api/v1/dhcp/leases \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{
    "vmid": 108,
    "hostname": "dhcp-vm",
    "ip": "193.34.77.10",
    "gateway": "193.34.77.1",
    "cidr": 24,
    "dns_servers": ["1.1.1.1", "8.8.8.8"],
    "lease_time_secs": 86400
  }'
```

## CI/CD Workflows

This repository includes GitHub Actions:

- CI: .github/workflows/ci.yml
  - runs cargo test and cargo check on push and PR
- Dev Build: .github/workflows/dev-build.yml
  - builds debug binary on dev branch pushes
  - uploads artifact feathertail-dev-linux-amd64
- Release Build: .github/workflows/release.yml
  - builds release binary on tags like v1.0.0
  - publishes asset feathertail-linux-amd64 to release

## Notes

- The daemon must run as root for DHCP UDP 67 and system integration tasks.
- FeatherTail exits early on non-Proxmox hosts by design.
