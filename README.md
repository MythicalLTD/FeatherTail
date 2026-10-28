# FeatherTail

Proxmox host agent for FeatherPanel. Installs **only via APT** on Proxmox VE nodes (`Depends: pve-manager`).

Provides DHCP lease management by VMID, Proxmox VM/LXC helpers, an HTTP API, and bundled noVNC assets under `/usr/share/novnc-pve`.

| Package | Channel | When |
|---------|---------|------|
| `feathertail` | **stable** | GitHub releases (`v*` tags) |
| `feathertail-dev` | **nightly** | Every commit to `main` |

These packages conflict; installing one replaces the other.

## Install (Proxmox host)

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates curl gnupg
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://apt.mythicalsystems.org/repository/keys/public.gpg \
  | sudo gpg --dearmor -o /etc/apt/keyrings/mythicalsystems.gpg
sudo chmod a+r /etc/apt/keyrings/mythicalsystems.gpg

ARCH="$(dpkg --print-architecture)"
echo "deb [arch=${ARCH} signed-by=/etc/apt/keyrings/mythicalsystems.gpg] https://apt.mythicalsystems.org/repository/MythicalSystems/ stable main" \
  | sudo tee /etc/apt/sources.list.d/mythicalsystems.list

sudo apt-get update
sudo apt-get install -y feathertail
# nightly:
# sudo apt-get install -y feathertail-dev
```

Or use the FeatherPanel installer on the Proxmox node (menu **[6] FeatherTail**, APT-only).

## After install

1. Edit `/etc/feathertail/feathertail.toml` — set a strong `api_token` and your DHCP settings.
2. Service is enabled on Proxmox hosts automatically; otherwise:

```bash
sudo systemctl enable --now feathertail
systemctl status feathertail
journalctl -u feathertail -f
```

## Upgrade

```bash
sudo apt-get update && sudo apt-get install --only-upgrade feathertail
# or
sudo apt-get update && sudo apt-get install --only-upgrade feathertail-dev
```

## Configuration

Default path: `/etc/feathertail/feathertail.toml` (`config|noreplace` on upgrade).

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

## API

Base path: `/api/v1` — docs at `/docs`, OpenAPI at `/openapi.json`.

- `GET/POST /dhcp/leases`, `DELETE /dhcp/leases/vm/{vmid}`
- `GET /servers`, `GET /containers`
- `POST /containers/{vmid}/root-password`
- `GET /proxmox/version`, `GET /proxmox/nodes`

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

## Notes

- Requires Proxmox VE (`pve-manager`). Non-Proxmox hosts cannot satisfy the dependency.
- Runs as root (DHCP UDP 67, system paths).
- Must be installed on the Proxmox **node**, not inside a guest VM without PVE packages.
