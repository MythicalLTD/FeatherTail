import { api, enc, ipToInt, refreshAll } from './api.ts'
import type { DhcpLease, PoolEntry } from './types.ts'

/** Lowest free pool IP on a bridge, or null. */
export function firstFree(entries: PoolEntry[]): PoolEntry | null {
  return entries.filter((e) => !e.lease).sort((a, b) => ipToInt(a.ip) - ipToInt(b.ip))[0] ?? null
}

/** One click: give the NIC the lowest free IP on its bridge. */
export async function autoAssign(vmid: number, mac: string, bridge: string, hostname?: string): Promise<DhcpLease> {
  const pool = await api.get<PoolEntry[]>(`/admin/pool?bridge=${enc(bridge)}`)
  const pick = firstFree(pool)
  if (!pick) throw new Error(`No free IPs on ${bridge}`)
  const lease = await api.post<DhcpLease>('/admin/leases', { vmid, mac, ip: pick.ip, hostname })
  refreshAll()
  return lease
}
