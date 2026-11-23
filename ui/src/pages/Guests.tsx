import { useMemo, useState } from 'react'
import { autoAssign } from '../actions.ts'
import { api, downloadCsv, errorMessage } from '../api.ts'
import { Copy, Empty, ErrorBox, PageHead, SearchInput, Seg, Skeleton, StatusPill, useLoader, useToast } from '../components.tsx'
import { navigate } from '../router.ts'
import type { GuestView, NicView, PageProps } from '../types.ts'

type Filter = 'all' | 'nolease' | 'running' | 'stopped'
type Kind = 'all' | 'qemu' | 'lxc'

export default function Guests({ params }: PageProps) {
  const toast = useToast()
  const { data, error, reload } = useLoader(() => api.get<GuestView[]>('/admin/guests'))
  const [search, setSearch] = useState(params.q || '')
  const [filter, setFilter] = useState<Filter>(params.filter === 'nolease' ? 'nolease' : 'all')
  const [kind, setKind] = useState<Kind>('all')
  const [busy, setBusy] = useState<string | null>(null)

  const guests = useMemo(() => (data || []).filter((g) => !g.template), [data])
  const missing = (g: GuestView) => g.nics.some((n) => !n.lease && n.bridge_has_pool)

  const rows = useMemo(() => {
    const q = search.trim().toLowerCase()
    return guests
      .filter((g) => kind === 'all' || g.kind === kind)
      .filter((g) => {
        if (filter === 'nolease') return missing(g)
        if (filter === 'running') return g.status === 'running'
        if (filter === 'stopped') return g.status === 'stopped'
        return true
      })
      .filter(
        (g) =>
          !q ||
          String(g.vmid).includes(q) ||
          g.name.toLowerCase().includes(q) ||
          g.nics.some((n) => n.mac.includes(q) || n.lease?.ip.includes(q) || n.bridge?.includes(q)),
      )
  }, [guests, search, filter, kind])

  const auto = async (g: GuestView, n: NicView) => {
    if (!n.bridge) return
    setBusy(n.mac)
    try {
      const lease = await autoAssign(g.vmid, n.mac, n.bridge, g.name)
      toast(
        <>
          <b className="font-monospace">{lease.ip}</b> → {g.vmid} {n.key}
        </>,
      )
    } catch (e) {
      toast(errorMessage(e), 'danger')
    } finally {
      setBusy(null)
    }
  }

  const autoAll = async () => {
    const todo = rows.flatMap((g) => g.nics.filter((n) => !n.lease && n.bridge_has_pool && n.free_ips_on_bridge > 0).map((n) => ({ g, n })))
    let ok = 0
    for (const { g, n } of todo) {
      try {
        await autoAssign(g.vmid, n.mac, n.bridge!, g.name)
        ok++
      } catch (e) {
        toast(`${g.vmid} ${n.key}: ${errorMessage(e)}`, 'danger')
      }
    }
    if (ok) toast(`${ok} assigned`)
  }

  const pending = rows.reduce((s, g) => s + g.nics.filter((n) => !n.lease && n.bridge_has_pool && n.free_ips_on_bridge > 0).length, 0)

  const exportCsv = () =>
    downloadCsv('feathertail-guests.csv', [
      ['vmid', 'name', 'kind', 'node', 'status', 'nic', 'mac', 'bridge', 'vlan', 'ip'],
      ...rows.flatMap((g) => g.nics.map((n) => [g.vmid, g.name, g.kind, g.node, g.status, n.key, n.mac, n.bridge, n.vlan, n.lease?.ip])),
    ])

  return (
    <>
      <PageHead
        title="Guests"
        meta={
          data && (
            <>
              <span className="badge text-bg-success">
                                {guests.filter((g) => g.status === 'running').length} running
              </span>
              <span className="badge border text-body">{guests.filter((g) => g.kind === 'qemu').length} VM</span>
              <span className="badge border text-body">{guests.filter((g) => g.kind === 'lxc').length} CT</span>
            </>
          )
        }
      >
        <button className="btn btn-outline-secondary" onClick={exportCsv} disabled={!rows.length}>
          <i className="bi bi-download" /> CSV
        </button>
        {pending > 0 && (
          <button className="btn btn-primary" onClick={autoAll}>
            <i className="bi bi-magic" /> Auto-assign {pending}
          </button>
        )}
      </PageHead>
      <ErrorBox error={error} onRetry={reload} />

      <div className="card">
        <div className="card-header d-flex align-items-center justify-content-between gap-2 flex-wrap">
          <SearchInput value={search} onChange={setSearch} placeholder="VMID, name, MAC, IP" />
          <div className="d-flex gap-2 flex-wrap">
            <Seg
              value={kind}
              onChange={setKind}
              options={[
                { value: 'all', label: 'All' },
                { value: 'qemu', label: 'VM', icon: 'pc-display' },
                { value: 'lxc', label: 'CT', icon: 'box' },
              ]}
            />
            <Seg
              value={filter}
              onChange={setFilter}
              options={[
                { value: 'all', label: 'Any', count: guests.length },
                { value: 'nolease', label: 'Needs IP', count: guests.filter(missing).length },
                { value: 'running', label: 'Running', count: guests.filter((g) => g.status === 'running').length },
                { value: 'stopped', label: 'Stopped', count: guests.filter((g) => g.status === 'stopped').length },
              ]}
            />
          </div>
        </div>

        {!data ? (
          <div className="p-3">
            <Skeleton rows={3} />
          </div>
        ) : (
          <div className="table-wrap">
            <table className="table align-middle">
              <thead>
                <tr>
                  <th>Guest</th>
                  <th>State</th>
                  <th>NIC</th>
                  <th>MAC</th>
                  <th>Bridge</th>
                  <th>Address</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((g) => {
                  const nics: (NicView | null)[] = g.nics.length ? g.nics : [null]
                  return nics.map((n, i) => (
                    <tr key={`${g.vmid}-${n?.key ?? 'none'}`}>
                      {i === 0 && (
                        <>
                          <td rowSpan={nics.length}>
                            <div className="d-flex align-items-center gap-2">
                              <div className="badge text-bg-secondary font-monospace">{g.kind === 'lxc' ? 'CT' : 'VM'}</div>
                              <div className="min-w-0">
                                <div className="fw-semibold">
                                  <span className="font-monospace">{g.vmid}</span> {g.name}
                                </div>
                                <div className="small text-body-secondary">{g.node}</div>
                              </div>
                            </div>
                          </td>
                          <td rowSpan={nics.length}>
                            <StatusPill status={g.status} />
                          </td>
                        </>
                      )}
                      {n ? (
                        <>
                          <td>
                            <span className="badge text-bg-secondary">{n.key}</span>
                          </td>
                          <td>
                            <Copy text={n.mac} className="font-monospace small text-body-secondary" />
                          </td>
                          <td className="font-monospace small">
                            {n.bridge || '—'}
                            {n.vlan && <span className="badge border text-body ms-1">vlan {n.vlan}</span>}
                          </td>
                          <td>
                            {n.lease ? (
                              <Copy text={n.lease.ip} className="font-monospace fw-semibold text-success" />
                            ) : n.bridge_has_pool ? (
                              <span className="badge text-bg-warning">none</span>
                            ) : (
                              <span className="text-body-secondary small">—</span>
                            )}
                          </td>
                          <td className="text-end text-nowrap">
                            {!n.lease && n.bridge_has_pool && n.free_ips_on_bridge > 0 && (
                              <button className="btn btn-sm btn-primary me-1" disabled={busy === n.mac} onClick={() => auto(g, n)}>
                                {busy === n.mac ? <span className="spinner-border spinner-border-sm" /> : <i className="bi bi-magic" />}
                                Auto
                              </button>
                            )}
                            <button
                              className="btn btn-sm btn-outline-secondary"
                              title={n.lease ? 'Change' : 'Pick'}
                              onClick={() => navigate('assign', { vmid: g.vmid, mac: n.mac })}
                            >
                              <i className={`bi bi-${n.lease ? 'arrow-left-right' : 'grid-3x3-gap'}`} />
                            </button>
                          </td>
                        </>
                      ) : (
                        <td colSpan={5} className="text-body-secondary small">
                          —
                        </td>
                      )}
                    </tr>
                  ))
                })}
              </tbody>
            </table>
            {rows.length === 0 && <Empty icon="pc-display">No matches</Empty>}
          </div>
        )}
      </div>
    </>
  )
}
