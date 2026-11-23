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

1. Prefer the FeatherPanel VDS node **FeatherTail** tab setup command (sets `api_token` via `FP_TAIL_TOKEN`). Otherwise edit `/etc/feathertail/feathertail.toml` — never leave `change-me`. DHCP stays **disabled** by default until you enable it intentionally.
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
enabled = false
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

### Setup wizard

```bash
sudo feathertail configure
```

The wizard asks whether the node is used with FeatherPanel, as a simple DHCP server only, or both. Then it asks for the token, the listen address, the DHCP on/off switch and the range (bridge, gateway, CIDR, first/last IP, DNS). It detects the bridges (`vmbr*`) and their IP addresses, so the defaults usually just need Enter. In DHCP-only mode it generates a token for you and prints it; that token is also your web UI login. The wizard edits `/etc/feathertail/feathertail.toml` in place, keeps your comments, and can restart the service when it is done.

### Simple DHCP (without FeatherPanel)

To use FeatherTail as a plain DHCP server, add one or more ranges. Any client that has no
lease then gets a free IP from the range automatically. You don't need FeatherPanel, the
admin UI or the API.

```toml
[dhcp]
enabled = true
lease_time_secs = 86400

[[dhcp.ranges]]
interface = "vmbr1"              # answer only on this bridge ("" = any interface)
start = "10.0.0.100"
end = "10.0.0.200"
gateway = "10.0.0.1"
cidr = 24
dns_servers = ["1.1.1.1", "8.8.8.8"]
# server_ip = "10.0.0.1"         # optional, defaults to gateway
# lease_time_secs = 3600         # optional, defaults to dhcp.lease_time_secs
```

Then run `sudo systemctl restart feathertail`.

- A client keeps the same IP across renewals and restarts while its old IP is still free.
- Static leases (from FeatherPanel, the API or the UI) always win. IPs held by static leases or listed in the UI **IP Pool** are never handed out dynamically.
- The gateway, the server IP and the network/broadcast addresses are skipped.
- An offered IP is held for 60 s. After the client's REQUEST, the lease runs for the full lease time. When a lease expires or the client releases it, the IP is free again.
- Dynamic leases appear in the UI and in `GET /dhcp/leases` with `static_lease: false`.
- Invalid ranges (for example, start and end in different subnets) are logged and skipped at startup.
- The gateway may be outside the client subnet, for example a routed `/28` like `212.87.213.113/28` via `212.87.212.1`. This works for static leases, the IP pool and ranges. The client keeps its real mask, and the reply also carries DHCP option 121/249 (classless static routes): `gateway/32` on-link plus the default route via the gateway.
- The `firewall_*` settings also apply to dynamic clients. With `allowlist`, only the listed MACs get an IP.

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

## Admin web UI

FeatherTail ships a small admin UI inside the binary. You can manage DHCP without FeatherPanel:

```
http://<node-ip>:8686/ui/
```

Log in with the daemon `api_token`, or set a separate admin password:

```bash
sudo feathertail set-password --config /etc/feathertail/feathertail.toml
# or non-interactive:
sudo FEATHERTAIL_ADMIN_PASSWORD='...' feathertail set-password --config /etc/feathertail/feathertail.toml
sudo systemctl restart feathertail
```

The UI has these pages:

- **Status**: listener state, lease and pool counts, every bridge with its pool usage.
- **Assign IP**: pick a VM/CT (ID + name), then one of its bridges (the MAC comes from the Proxmox config), then a free IP on that bridge.
- **DHCP Leases**: search, change IP, delete, or add a manual lease for any MAC.
- **IP Pool**: you decide which IPs are handed out on which bridge (`vmbr0` public IPs, `vmbr1` 10.0.0.x, ...). Add single IPs, lists or ranges (`10.0.0.40-10.0.0.100`, `1.2.3.4-8`), each with its gateway, CIDR and DNS.
- **VMs & CTs**: every NIC with its MAC, bridge and lease.
- **Settings**: turn the DHCP listener on or off, set the default server IP and lease time, restart the service, change the password, view logs.

FeatherTail **never changes host networking**: no bridges, no addresses, no NAT or iptables. You set up the bridges yourself (e.g. `vmbr1` with `10.0.0.1/24` on the host). FeatherTail answers DHCP and sends each reply out of the interface the request came in on. An IP from the pool is only handed out on the bridge you put it on. The DHCP server identifier is the pool entry's *server IP*, or its gateway when that field is empty.

Sessions are a `HttpOnly; SameSite=Strict` cookie and expire after 12 hours. The UI is plain HTTP on the API port, so firewall `8686` to trusted IPs or put it behind a TLS reverse proxy.

Building from source needs Node.js/npm; `build.rs` runs `npm ci && npm run build` in `ui/`. Set `FEATHERTAIL_SKIP_UI_BUILD=1` to skip that step and embed a placeholder page. For UI development, run `cd ui && FT_API=http://<node>:8686 npm run dev`.

The admin API is under `/api/v1/admin/*` and `/api/v1/auth/*`. It accepts the session cookie or `Authorization: Bearer <api_token>`.

## Notes

- Requires Proxmox VE (`pve-manager`). Non-Proxmox hosts cannot satisfy the dependency.
- Runs as root (DHCP UDP 67, system paths).
- Must be installed on the Proxmox **node**, not inside a guest VM without PVE packages.
