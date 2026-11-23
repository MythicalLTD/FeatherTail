import { useState } from 'react'
import { autoAssign } from '../actions.ts'
import { api, errorMessage, fmtRelative } from '../api.ts'
import { Copy, Dot, Empty, ErrorBox, Meter, PageHead, Skeleton, StatusPill, Tile, useLoader, useToast } from '../components.tsx'
import { navigate } from '../router.ts'
import type { GuestView, LeaseView, NicView, Overview } from '../types.ts'

export default function Status() {
  const overview = useLoader(() => api.get<Overview>('/admin/overview'))
  const guests = useLoader(() => api.get<GuestView[]>('/admin/guests'))
  const leases = useLoader(() => api.get<LeaseView[]>('/admin/leases'))
  const data = overview.data

  if (!data) {
    return overview.error ? (
      <ErrorBox error={overview.error} onRetry={overview.reload} />
    ) : (
      <div className="row g-3">
        {[0, 1, 2, 3].map((i) => (
          <div className="col-6 col-xl-3" key={i}>
            <Skeleton rows={1} />
          </div>
        ))}
      </div>
    )
  }

  const poolUsed = data.bridges.reduce((s, b) => s + b.pool_used, 0)
  const poolTotal = data.bridges.reduce((s, b) => s + b.pool_total, 0)

  return (
    <>
      <PageHead
        title="Overview"
        meta={
          <>
            <span className={`badge ${data.dhcp_listener_enabled ? 'text-bg-success' : 'text-bg-secondary'}`}>
                            DHCP {data.dhcp_listener_enabled ? 'serving' : 'off'}
            </span>
            <span className="badge border text-body font-monospace">{data.dhcp_bind}</span>
            <span className="badge border text-body">server {data.dhcp_server_ip}</span>
          </>
        }
      >
        <button className="btn btn-primary" onClick={() => navigate('assign')}>
          <i className="bi bi-plus-lg" /> Assign IP
        </button>
      </PageHead>

      {!data.dhcp_listener_enabled && (
        <div className="alert alert-warning d-flex align-items-center gap-2">
          <i className="bi bi-pause-circle" />
          <span className="me-auto">Listener disabled</span>
          <a className="btn btn-sm btn-outline-secondary" href="#settings">
            Settings
          </a>
        </div>
      )}
      {data.token_is_default && (
        <div className="alert alert-danger">
          <i className="bi bi-shield-exclamation me-2" />
          <code>api_token</code> is <code>change-me</code>
        </div>
      )}

      <div className="row g-3 mb-4">
        <div className="col-6 col-xl-3">
          <Tile
            label="Leases"
            icon="hdd-network"
            value={data.lease_count}
            hint={`${data.active_lease_count} active`}
            onClick={() => navigate('leases')}
          />
        </div>
        <div className="col-6 col-xl-3">
          <Tile
            label="Pool utilization"
            icon="pie-chart"
            value={
              <>
                {poolTotal ? Math.round((poolUsed / poolTotal) * 100) : 0}
                <span className="fs-6 text-body-secondary">%</span>
              </>
            }
            hint={
              <>
                <Meter value={poolUsed} max={poolTotal} />
                <div className="mt-1">
                  {poolUsed} of {poolTotal}
                </div>
              </>
            }
            onClick={() => navigate('pool')}
          />
        </div>
        <div className="col-6 col-xl-3">
          <Tile
            label="Guests"
            icon="pc-display"
            value={data.guest_count}
            hint={`${data.bridges.length} bridges`}
            onClick={() => navigate('guests')}
          />
        </div>
        <div className="col-6 col-xl-3">
          <Tile
            label="Without IP"
            icon="exclamation-diamond"
            value={<span className={data.nics_without_lease ? 'text-warning' : 'text-success'}>{data.nics_without_lease}</span>}
            hint={data.nics_without_lease ? 'on bridges with a pool' : 'all set'}
            onClick={() => navigate('guests', { filter: 'nolease' })}
          />
        </div>
      </div>

      <div className="d-flex align-items-center justify-content-between mb-2">
        <h6 className="mb-0 fw-semibold">Bridges</h6>
        <a href="#pool" className="small">
          Address pool <i className="bi bi-arrow-right" />
        </a>
      </div>
      <div className="row g-3 mb-4">
        {data.bridges.length === 0 && (
          <div className="col-12">
            <div className="card">
              <Empty icon="diagram-3">No bridges</Empty>
            </div>
          </div>
        )}
        {data.bridges.map((b) => {
          const tone = !b.present ? 'warning' : b.state === 'up' || b.state === 'unknown' ? 'success' : 'secondary'
          return (
            <div className="col-md-6 col-xl-4" key={b.name}>
              <div className="card h-100 cursor-pointer" onClick={() => navigate('pool', { bridge: b.name })}>
                <div className="card-body">
                <div className="d-flex align-items-center gap-2 mb-1">
                  <Dot tone={tone} />
                  <span className="font-monospace fw-bold">{b.name}</span>
                  {!b.present && <span className="badge text-bg-warning ms-auto">missing</span>}
                  {b.present && b.state && <span className="badge text-bg-secondary ms-auto">{b.state}</span>}
                </div>
                <div className="font-monospace small text-body-secondary mb-3 text-truncate">{b.host_addresses.join(', ') || 'no address'}</div>
                <div className="d-flex align-items-end justify-content-between mb-2">
                  <div>
                    <span className="fs-4 fw-semibold">{b.pool_free}</span> <span className="text-body-secondary small">free</span>
                  </div>
                  <div className="text-body-secondary small">
                    {b.pool_used}/{b.pool_total}
                  </div>
                </div>
                <Meter value={b.pool_used} max={b.pool_total} />
                <div className="d-flex justify-content-between small text-body-secondary mt-3">
                  <span>
                    <i className="bi bi-ethernet me-1" />
                    {b.guest_nics} NICs
                  </span>
                  {b.guest_nics_without_lease > 0 && b.pool_total > 0 ? (
                    <span className="text-warning">{b.guest_nics_without_lease} without IP</span>
                  ) : (
                    <span className="text-body-secondary">—</span>
                  )}
                </div>
                </div>
              </div>
            </div>
          )
        })}
      </div>

      <div className="row g-3">
        <div className="col-xl-7">
          <NeedsIp guests={guests.data} />
        </div>
        <div className="col-xl-5">
          <RecentLeases leases={leases.data} />
        </div>
      </div>
    </>
  )
}

function NeedsIp({ guests }: { guests: GuestView[] | null }) {
  const toast = useToast()
  const [busy, setBusy] = useState<string | null>(null)
  const rows: { g: GuestView; n: NicView }[] = []
  for (const g of guests ?? []) {
    if (g.template) continue
    for (const n of g.nics) if (!n.lease && n.bridge_has_pool && n.free_ips_on_bridge > 0) rows.push({ g, n })
  }

  const auto = async (g: GuestView, n: NicView) => {
    if (!n.bridge) return
    setBusy(n.mac)
    try {
      const lease = await autoAssign(g.vmid, n.mac, n.bridge, g.name)
      toast(
        <>
          <b className="font-monospace">{lease.ip}</b> → {g.vmid} {g.name}
        </>,
      )
    } catch (e) {
      toast(errorMessage(e), 'danger')
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="card h-100">
      <div className="card-header d-flex align-items-center justify-content-between gap-2">
        <span className="d-flex align-items-center gap-2">
          <i className="bi bi-lightning-charge" /> Suggestions
        </span>
        {rows.length > 0 && <span className="badge text-bg-warning">{rows.length}</span>}
      </div>
      {!guests ? (
        <Skeleton rows={2} />
      ) : rows.length === 0 ? (
        <Empty icon="check2-circle">Every NIC has an address</Empty>
      ) : (
        <div className="list-group list-group-flush">
          {rows.slice(0, 8).map(({ g, n }) => (
            <div className="list-group-item d-flex align-items-center gap-2" key={n.mac}>
              <div className="badge text-bg-secondary font-monospace">{g.kind === 'lxc' ? 'CT' : 'VM'}</div>
              <div className="min-w-0 flex-grow-1">
                <div className="fw-semibold text-truncate">
                  <span className="font-monospace">{g.vmid}</span> {g.name}
                </div>
                <div className="small text-body-secondary font-monospace text-truncate">
                  {n.key} · {n.bridge} · {n.mac}
                </div>
              </div>
              <StatusPill status={g.status} />
              <button className="btn btn-sm btn-outline-secondary" onClick={() => navigate('assign', { vmid: g.vmid, mac: n.mac })} title="Pick an IP">
                <i className="bi bi-grid-3x3-gap" />
              </button>
              <button className="btn btn-sm btn-primary" disabled={busy === n.mac} onClick={() => auto(g, n)} title="Next free IP">
                {busy === n.mac ? <span className="spinner-border spinner-border-sm" /> : <i className="bi bi-magic" />}
                Auto
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function RecentLeases({ leases }: { leases: LeaseView[] | null }) {
  const recent = [...(leases ?? [])].sort((a, b) => b.lease_start - a.lease_start).slice(0, 7)
  return (
    <div className="card h-100">
      <div className="card-header d-flex align-items-center justify-content-between gap-2">
        <span className="d-flex align-items-center gap-2">
          <i className="bi bi-clock-history" /> Recent leases
        </span>
        <a href="#leases" className="small fw-normal">
          All <i className="bi bi-arrow-right" />
        </a>
      </div>
      {!leases ? (
        <Skeleton rows={2} />
      ) : recent.length === 0 ? (
        <Empty icon="hdd-network">No leases</Empty>
      ) : (
        <div className="list-group list-group-flush">
        {recent.map((l) => (
          <div className="list-group-item d-flex align-items-center gap-2" key={l.mac}>
            <Dot tone={l.state === 'active' ? 'success' : 'secondary'} />
            <div className="min-w-0 flex-grow-1">
              <Copy text={l.ip} className="font-monospace fw-semibold" />
              <div className="small text-body-secondary text-truncate">
                {l.vmid != null ? `${l.vmid} · ${l.guest_name ?? l.hostname ?? ''}` : l.hostname || l.mac}
              </div>
            </div>
            <span className="badge text-bg-secondary font-monospace">{l.nic_bridge || l.pool_bridge || '—'}</span>
            <span className="small text-body-secondary" style={{ minWidth: 64, textAlign: 'right' }}>
              {fmtRelative(l.lease_start)}
            </span>
          </div>
        ))}
        </div>
      )}
    </div>
  )
}
