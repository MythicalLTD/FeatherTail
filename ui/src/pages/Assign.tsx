import { useEffect, useMemo, useState } from 'react'
import { firstFree } from '../actions.ts'
import { api, enc, refreshAll } from '../api.ts'
import { Dot, Empty, ErrorBox, PageHead, SearchInput, Seg, Skeleton, StatusPill, useLoader, useToast } from '../components.tsx'
import { navigate } from '../router.ts'
import type { DhcpLease, GuestView, NicView, PageProps, PoolEntry } from '../types.ts'

type GuestFilter = 'all' | 'nolease' | 'running'

export default function Assign({ params }: PageProps) {
  const toast = useToast()
  const guests = useLoader(() => api.get<GuestView[]>('/admin/guests'))
  const [vmid, setVmid] = useState<number | null>(params.vmid ? Number(params.vmid) : null)
  const [mac, setMac] = useState<string | null>(params.mac || null)
  const [ip, setIp] = useState<string | null>(null)
  const [hostname, setHostname] = useState('')
  const [search, setSearch] = useState('')
  const [filter, setFilter] = useState<GuestFilter>('all')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)

  const guest = guests.data?.find((g) => g.vmid === vmid)
  const nic = guest?.nics.find((n) => n.mac === mac)
  const bridge = nic?.bridge ?? null
  const step = !guest ? 0 : !nic ? 1 : 2

  useEffect(() => {
    if (guest && !mac && guest.nics.length === 1) setMac(guest.nics[0].mac)
    if (guest) setHostname((h) => h || guest.name)
  }, [guest, mac])

  const pool = useLoader<PoolEntry[]>(
    () => (bridge ? api.get<PoolEntry[]>(`/admin/pool?bridge=${enc(bridge)}`) : Promise.resolve([])),
    [bridge],
  )

  const visible = useMemo(() => (guests.data || []).filter((g) => !g.template), [guests.data])
  const needsIp = (g: GuestView) => g.nics.some((n) => !n.lease)
  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase()
    return visible
      .filter((g) => filter === 'all' || (filter === 'nolease' ? needsIp(g) : g.status === 'running'))
      .filter(
        (g) =>
          !q ||
          String(g.vmid).includes(q) ||
          g.name.toLowerCase().includes(q) ||
          g.node.toLowerCase().includes(q) ||
          g.nics.some((n) => n.mac.includes(q) || n.bridge?.includes(q)),
      )
  }, [visible, search, filter])

  const pickVm = (g: GuestView) => {
    setVmid(g.vmid)
    setMac(g.nics.length === 1 ? g.nics[0].mac : null)
    setIp(null)
    setHostname(g.name)
    setError(null)
  }

  const submit = async () => {
    if (!ip) return
    setBusy(true)
    setError(null)
    try {
      const lease = await api.post<DhcpLease>('/admin/leases', { vmid, mac, ip, hostname })
      toast(
        <>
          <b className="font-monospace">{lease.ip}</b> → {vmid} on <span className="font-monospace">{bridge}</span>
        </>,
      )
      refreshAll()
      navigate('leases', { q: lease.ip })
    } catch (e) {
      setError(e)
    } finally {
      setBusy(false)
    }
  }

  const steps = [
    { label: 'Guest', value: guest ? `${guest.vmid} · ${guest.name}` : null, back: () => { setVmid(null); setMac(null); setIp(null) } },
    { label: 'Interface', value: nic ? `${nic.bridge ?? '?'} · ${nic.key}` : null, back: () => { setMac(null); setIp(null) } },
    { label: 'Address', value: ip, back: () => setIp(null) },
  ]

  return (
    <>
      <PageHead title="Assign IP" />
      <ErrorBox error={guests.error} onRetry={guests.reload} />

      <div className="list-group list-group-horizontal-md mb-4">
        {steps.map((s, i) => (
          <button
            type="button"
            key={s.label}
            className={`list-group-item list-group-item-action flex-fill d-flex align-items-center gap-3 ${i === step ? 'active' : ''}`}
            disabled={i > step}
            onClick={i < step ? s.back : undefined}
          >
            <span className={`badge rounded-pill ${i < step ? 'text-bg-success' : i === step ? 'text-bg-light' : 'text-bg-secondary'}`}>
              {i < step ? <i className="bi bi-check-lg" /> : i + 1}
            </span>
            <div className="min-w-0 text-start">
              <div className="small">{s.label}</div>
              {s.value && <div className="font-monospace small fw-semibold text-truncate">{s.value}</div>}
            </div>
          </button>
        ))}
      </div>

      {/* step 1 */}
      {step === 0 && (
        <div className="card">
          <div className="card-header d-flex align-items-center justify-content-between gap-2">
            <SearchInput value={search} onChange={setSearch} placeholder="VMID, name, node, MAC" />
            <Seg
              value={filter}
              onChange={setFilter}
              options={[
                { value: 'all', label: 'All', count: visible.length },
                { value: 'nolease', label: 'Needs IP', count: visible.filter(needsIp).length },
                { value: 'running', label: 'Running', count: visible.filter((g) => g.status === 'running').length },
              ]}
            />
          </div>
          {!guests.data ? (
            <div className="p-3">
              <Skeleton rows={3} />
            </div>
          ) : (
            <div className="table-wrap" style={{ maxHeight: 560 }}>
              <table className="table table-hover align-middle">
                <thead>
                  <tr>
                    <th style={{ width: 90 }}>ID</th>
                    <th>Name</th>
                    <th>Node</th>
                    <th>State</th>
                    <th>Interfaces</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((g) => (
                    <tr key={g.vmid} className="pick-row" onClick={() => pickVm(g)}>
                      <td>
                        <span className="font-monospace fw-semibold">{g.vmid}</span>
                      </td>
                      <td>
                        <div className="d-flex align-items-center gap-2">
                          <i className={`bi bi-${g.kind === 'lxc' ? 'box' : 'pc-display'} text-body-secondary`} />
                          <span className="fw-medium">{g.name}</span>
                        </div>
                      </td>
                      <td className="text-body-secondary">{g.node}</td>
                      <td>
                        <StatusPill status={g.status} />
                      </td>
                      <td>
                        <div className="d-flex flex-wrap gap-1">
                          {g.nics.length === 0 && <span className="text-body-secondary small">—</span>}
                          {g.nics.map((n) => (
                            <span key={n.key} className={`badge ${n.lease ? 'text-bg-success' : 'text-bg-warning'} font-monospace`}>
                              {n.bridge || '?'} {n.lease ? n.lease.ip : '·'}
                            </span>
                          ))}
                        </div>
                      </td>
                      <td className="text-end text-body-secondary">
                        <i className="bi bi-chevron-right" />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {filtered.length === 0 && <Empty icon="search">No matches</Empty>}
            </div>
          )}
        </div>
      )}

      {/* step 2 */}
      {step === 1 && guest && (
        <div className="row g-3">
          {guest.nics.length === 0 && (
            <div className="col-12">
              <div className="card">
                <Empty icon="ethernet">No network interfaces</Empty>
              </div>
            </div>
          )}
          {guest.nics.map((n) => (
            <div className="col-md-6 col-xl-4" key={n.key}>
              <NicOption nic={n} onPick={() => { setMac(n.mac); setIp(null) }} />
            </div>
          ))}
        </div>
      )}

      {/* step 3 */}
      {step === 2 && guest && nic && (
        <div className="card">
          <div className="card-header d-flex align-items-center justify-content-between gap-2">
            <span className="d-flex align-items-center gap-2">
              <i className="bi bi-grid-3x3-gap" /> <span className="font-monospace">{bridge}</span>
            </span>
            {pool.data && pool.data.length > 0 && (
              <button
                className="btn btn-sm btn-outline-secondary"
                onClick={() => setIp(firstFree(pool.data!.filter((e) => !e.lease || e.lease.mac === nic.mac))?.ip ?? null)}
              >
                <i className="bi bi-magic" /> Suggest
              </button>
            )}
          </div>
          <div className="card-body">
            {nic.lease && (
              <div className="alert alert-info py-2">
                <i className="bi bi-arrow-left-right me-2" />
                Current: <b className="font-monospace">{nic.lease.ip}</b>
              </div>
            )}
            {guest.kind === 'lxc' && nic.ip_config && nic.ip_config !== 'dhcp' && (
              <div className="alert alert-warning py-2">
                <i className="bi bi-exclamation-triangle me-2" />
                <span className="font-monospace">
                  {nic.key}: ip={nic.ip_config}
                </span>
              </div>
            )}
            {pool.loading && !pool.data ? (
              <Skeleton rows={1} />
            ) : pool.data?.length ? (
              <IpMap entries={pool.data} selected={ip} currentMac={nic.mac} onPick={setIp} />
            ) : bridge ? (
              <Empty icon="diagram-3">
                <div className="mb-3">
                  No addresses on <span className="font-monospace">{bridge}</span>
                </div>
                <button className="btn btn-primary" onClick={() => navigate('pool', { bridge, add: 1 })}>
                  <i className="bi bi-plus-lg" /> Add range
                </button>
              </Empty>
            ) : (
              <Empty icon="ethernet">No bridge</Empty>
            )}
          </div>
          {ip && (
            <div className="card-footer p-3">
              <div className="d-flex flex-wrap align-items-end gap-3">
                <div style={{ minWidth: 220 }}>
                  <label className="form-label">Hostname</label>
                  <input className="form-control" value={hostname} onChange={(e) => setHostname(e.target.value)} />
                </div>
                <div className="ms-auto d-flex align-items-center gap-3 flex-wrap">
                  <div className="font-monospace small text-body-secondary">
                    {nic.mac} <i className="bi bi-arrow-right mx-1" />
                    <b className="text-body fs-6">{ip}</b>
                  </div>
                  <button className="btn btn-primary" onClick={submit} disabled={busy}>
                    {busy ? <span className="spinner-border spinner-border-sm" /> : <i className="bi bi-check2" />}
                    Assign
                  </button>
                </div>
              </div>
              <div className="mt-3">
                <ErrorBox error={error} />
              </div>
            </div>
          )}
        </div>
      )}
    </>
  )
}

function NicOption({ nic, onPick }: { nic: NicView; onPick: () => void }) {
  return (
    <button className="btn btn-outline-secondary w-100 text-start p-3" onClick={onPick}>
      <div className="d-flex align-items-center gap-2 mb-2">
        <i className="bi bi-ethernet text-body-secondary" />
        <span className="font-monospace fw-bold fs-6">{nic.bridge || '—'}</span>
        <span className="badge text-bg-secondary ms-auto">{nic.key}</span>
      </div>
      <div className="font-monospace small text-body-secondary mb-2">
        {nic.mac}
        {nic.vlan ? ` · vlan ${nic.vlan}` : ''}
        {nic.model ? ` · ${nic.model}` : ''}
      </div>
      {nic.lease ? (
        <span className="badge text-bg-success font-monospace">{nic.lease.ip}</span>
      ) : nic.bridge_has_pool ? (
        <span className="badge text-bg-primary">{nic.free_ips_on_bridge} free</span>
      ) : (
        <span className="badge text-bg-secondary">no pool</span>
      )}
    </button>
  )
}

type MapFilter = 'all' | 'free' | 'used'

function IpMap({ entries, selected, currentMac, onPick }: { entries: PoolEntry[]; selected: string | null; currentMac: string; onPick: (ip: string) => void }) {
  const [filter, setFilter] = useState<MapFilter>('all')
  const [q, setQ] = useState('')
  const suggested = firstFree(entries)?.ip
  const free = entries.filter((e) => !e.lease).length
  const shown = entries
    .filter((e) => filter === 'all' || (filter === 'free' ? !e.lease : !!e.lease))
    .filter((e) => !q || e.ip.includes(q) || e.notes?.toLowerCase().includes(q.toLowerCase()))

  return (
    <>
      <div className="d-flex flex-wrap gap-2 align-items-center mb-3">
        <SearchInput value={q} onChange={setQ} placeholder="Filter" />
        <Seg
          value={filter}
          onChange={setFilter}
          options={[
            { value: 'all', label: 'All', count: entries.length },
            { value: 'free', label: 'Free', count: free },
            { value: 'used', label: 'Used', count: entries.length - free },
          ]}
        />
        <div className="ms-auto d-flex gap-3 small text-body-secondary">
          <span>
            <Dot tone="success" />
            free
          </span>
          <span>
            <Dot tone="primary" />
            leased
          </span>
          <span>
            <Dot tone="warning" />
            this NIC
          </span>
          <span className="font-monospace">
            gw {entries[0].gateway}/{entries[0].cidr}
          </span>
        </div>
      </div>
      <div className="ip-grid">
        {shown.map((e) => {
          const holder = e.lease
          const mine = holder?.mac === currentMac
          const usedBy = holder && !mine ? holder : null
          return (
            <button
              key={e.ip}
              disabled={!!usedBy}
              className={`btn btn-sm btn-outline-${usedBy ? 'primary' : mine ? 'warning' : 'success'} ${selected === e.ip ? 'active' : ''}`}
              onClick={() => onPick(e.ip)}
              title={usedBy ? `${usedBy.mac}` : e.notes || ''}
            >
              <div className="d-flex align-items-center">
                <span className="font-monospace fw-semibold">{e.ip}</span>
                {e.ip === suggested && !selected && <span className="badge text-bg-success ms-auto">best</span>}
              </div>
              <div className="sub">
                {mine ? 'current' : usedBy ? `${usedBy.vmid ?? ''} ${usedBy.guest_name || usedBy.hostname || usedBy.mac}` : e.notes || 'free'}
              </div>
            </button>
          )
        })}
      </div>
      {shown.length === 0 && <Empty icon="funnel">Nothing here</Empty>}
    </>
  )
}
