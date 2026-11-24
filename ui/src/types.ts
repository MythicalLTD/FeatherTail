// Shapes returned by /api/v1/admin/* and /api/v1/auth/* (see src/routes/api/admin.rs).

export interface SessionInfo {
  authenticated: boolean
  password_configured: boolean
  method: string | null
}

export interface DhcpLease {
  mac: string
  ip: string
  hostname: string | null
  vmid: number | null
  node: string | null
  gateway: string
  cidr: number
  dns_servers: string[]
  lease_start: number
  lease_end: number
  state: string
  static_lease: boolean
}

export interface BridgeSummary {
  name: string
  present: boolean
  state: string | null
  host_addresses: string[]
  pool_total: number
  pool_used: number
  pool_free: number
  guest_nics: number
  guest_nics_without_lease: number
}

export interface Overview {
  daemon: string
  version: string
  hostname: string | null
  api_bind: string
  dhcp_listener_enabled: boolean
  dhcp_server_ip: string
  dhcp_bind: string
  lease_count: number
  active_lease_count: number
  pool_count: number
  guest_count: number
  nics_without_lease: number
  bridges: BridgeSummary[]
  password_configured: boolean
  token_is_default: boolean
}

export interface NicView {
  key: string
  mac: string
  bridge: string | null
  model: string | null
  vlan: string | null
  ip_config: string | null
  lease: DhcpLease | null
  bridge_has_pool: boolean
  free_ips_on_bridge: number
}

export interface GuestView {
  vmid: number
  kind: 'qemu' | 'lxc'
  name: string
  node: string
  status: string | null
  template: boolean
  nics: NicView[]
  tags: string[]
}

export interface PoolHolder {
  mac: string
  vmid: number | null
  hostname: string | null
  state: string
  guest_name: string | null
}

export interface PoolEntry {
  ip: string
  bridge: string
  gateway: string
  cidr: number
  dns_servers: string[]
  server_ip: string | null
  notes: string | null
  lease: PoolHolder | null
}

export interface PoolInput {
  ip?: string
  bridge: string
  gateway: string
  cidr: number
  dns_servers: string[]
  server_ip: string | null
  notes: string | null
}

export interface AddPoolRequest extends PoolInput {
  ips: string
}

export interface AddPoolResponse {
  added: number
  skipped: string[]
  moved_from_other_bridge: string[]
}

export interface LeaseView extends DhcpLease {
  guest_name: string | null
  guest_status: string | null
  nic_key: string | null
  nic_bridge: string | null
  pool_bridge: string | null
  last_offer_at: number | null
  last_ack_at: number | null
  acked_ip: string | null
}

export interface LeaseCheck {
  mac: string
  ip: string
  ping_ok: boolean
  ping_ms: number | null
  neighbor_mac: string | null
  mac_matches: boolean | null
  checked_at: number
}

export interface CreateLeaseRequest {
  vmid?: number | null
  mac?: string | null
  ip: string
  hostname?: string
  gateway?: string
  cidr?: number
  dns_servers?: string[]
  lease_time_secs?: number
  force?: boolean
}

export interface SettingsView {
  dhcp_enabled: boolean
  dhcp_bind: string
  dhcp_server_ip: string
  dhcp_lease_time_secs: number
  api_bind: string
  config_path: string
  password_configured: boolean
  running_dhcp_enabled: boolean
}

export interface UpdateDhcpSettings {
  enabled: boolean
  server_ip: string
  lease_time_secs: number
}

export interface LogsResponse {
  content: string
}

/** Hash-route query params, e.g. `#assign?vmid=101&mac=...`. */
export type RouteParams = Record<string, string>

export interface PageProps {
  params: RouteParams
}
