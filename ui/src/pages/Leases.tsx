import { useEffect, useMemo, useRef, useState, type ChangeEvent, type FormEvent, type ReactNode } from 'react'
import { api, downloadCsv, enc, errorMessage, fmtRelative, fmtTime, ipToInt, refreshAll, splitList } from '../api.ts'
import { Copy, Empty, ErrorBox, Modal, PageHead, SearchInput, Seg, Skeleton, StatusPill, useConfirm, useLoader, useToast } from '../components.tsx'
import { navigate } from '../router.ts'
import type { CreateLeaseRequest, DhcpLease, LeaseCheck, LeaseView, PageProps } from '../types.ts'

export default function Leases({ params }: PageProps) {
  const toast = useToast()
  const confirm = useConfirm()
  const { data, error, reload } = useLoader(() => api.get<LeaseView[]>('/admin/leases'))
  const [search, setSearch] = useState(params.q || '')
  const [bridge, setBridge] = useState('all')
  const [manual, setManual] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())

  const bridgeOf = (l: LeaseView) => l.nic_bridge || l.pool_bridge || '—'
  const bridges = useMemo(() => [...new Set((data || []).map(bridgeOf))].sort(), [data])

  const rows = useMemo(() => {
    const q = search.trim().toLowerCase()
    return (data || [])
      .filter((l) => bridge === 'all' || bridgeOf(l) === bridge)
      .filter(
        (l) =>
          !q ||
          [l.ip, l.mac, l.hostname, l.guest_name, String(l.vmid ?? ''), l.pool_bridge, l.nic_bridge]
            .filter((v): v is string => !!v)
            .some((v) => v.toLowerCase().includes(q)),
      )
      .sort((a, b) => ipToInt(a.ip) - ipToInt(b.ip))
  }, [data, search, bridge])

  const removeMany = async (list: LeaseView[]) => {
    const ok = await confirm(
      list.length === 1 ? (
        <>
          Release <b className="font-monospace">{list[0].ip}</b> from <span className="font-monospace">{list[0].mac}</span>
          {list[0].vmid != null && <> ({list[0].vmid} {list[0].guest_name})</>}?
        </>
      ) : (
        <>Release {list.length} leases?</>
      ),
      { title: 'Release lease', confirmLabel: 'Release' },
    )
    if (!ok) return
    try {
      for (const l of list) await api.del(`/admin/leases/${enc(l.mac)}`)
      toast(list.length === 1 ? <>Released <span className="font-monospace">{list[0].ip}</span></> : `Released ${list.length} leases`)
    } catch (e) {
      toast(errorMessage(e), 'danger')
    }
    setSelected(new Set())
    refreshAll()
  }

  const [checks, setChecks] = useState<Record<string, LeaseCheck | 'pending'>>({})
  const checkOne = async (l: LeaseView) => {
    setChecks((c) => ({ ...c, [l.mac]: 'pending' }))
    try {
      const res = await api.post<LeaseCheck>(`/admin/leases/${enc(l.mac)}/check`, {})
      setChecks((c) => ({ ...c, [l.mac]: res }))
    } catch (e) {
      setChecks((c) => {
        const next = { ...c }
        delete next[l.mac]
        return next
      })
      toast(<>{l.ip}: {errorMessage(e)}</>, 'danger')
    }
  }
  // Probe every active lease once when the page opens.
  const autoChecked = useRef(false)
  useEffect(() => {
    if (!data || autoChecked.current) return
    autoChecked.current = true
    data.filter((l) => l.state === 'active').forEach((l) => void checkOne(l))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data])

  const renewMany = async (list: LeaseView[]) => {
    let failed = 0
    for (const l of list) {
      try {
        await api.post(`/admin/leases/${enc(l.mac)}/renew`, {})
      } catch (e) {
        failed++
        toast(<>{l.ip}: {errorMessage(e)}</>, 'danger')
      }
    }
    const ok = list.length - failed
    if (ok > 0) toast(list.length === 1 ? <>Renewed <span className="font-monospace">{list[0].ip}</span></> : `Renewed ${ok} leases`)
    setSelected(new Set())
    refreshAll()
  }

  const exportCsv = () =>
    downloadCsv('feathertail-leases.csv', [
      ['ip', 'mac', 'vmid', 'guest', 'hostname', 'bridge', 'gateway', 'cidr', 'dns', 'state', 'lease_end'],
      ...rows.map((l) => [
        l.ip,
        l.mac,
        l.vmid,
        l.guest_name,
        l.hostname,
        bridgeOf(l),
        l.gateway,
        l.cidr,
        l.dns_servers.join(' '),
        l.state,
        new Date(l.lease_end * 1000).toISOString(),
      ]),
    ])

  const toggle = (mac: string) => {
    const next = new Set(selected)
    if (next.has(mac)) next.delete(mac)
    else next.add(mac)
    setSelected(next)
  }
  const allChecked = rows.length > 0 && rows.every((r) => selected.has(r.mac))

  return (
    <>
      <PageHead
        title="Leases"
        meta={
          data && (
            <>
              <span className="badge text-bg-success">
                                {data.filter((l) => l.state === 'active').length} active
              </span>
              <span className="badge border text-body">{data.length} total</span>
            </>
          )
        }
      >
        <button className="btn btn-outline-secondary" onClick={exportCsv} disabled={!rows.length}>
          <i className="bi bi-download" /> CSV
        </button>
        <button className="btn btn-outline-secondary" onClick={() => setManual(true)}>
          <i className="bi bi-pencil" /> Manual
        </button>
        <button className="btn btn-primary" onClick={() => navigate('assign')}>
          <i className="bi bi-plus-lg" /> Assign IP
        </button>
      </PageHead>
      <ErrorBox error={error} onRetry={reload} />

      <div className="card">
        <div className="card-header d-flex align-items-center justify-content-between gap-2 flex-wrap">
          <SearchInput value={search} onChange={setSearch} placeholder="IP, MAC, VMID, hostname" />
          <Seg
            value={bridge}
            onChange={setBridge}
            options={[
              { value: 'all', label: 'All', count: data?.length ?? 0 },
              ...bridges.map((b) => ({ value: b, label: <span className="font-monospace">{b}</span>, count: (data || []).filter((l) => bridgeOf(l) === b).length })),
            ]}
          />
        </div>
        {!data ? (
          <div className="p-3">
            <Skeleton rows={3} />
          </div>
        ) : (
          <div className="table-wrap">
            <table className="table table-hover align-middle">
              <thead>
                <tr>
                  <th style={{ width: 36 }}>
                    <input
                      type="checkbox"
                      className="form-check-input"
                      checked={allChecked}
                      onChange={(e) => setSelected(e.target.checked ? new Set(rows.map((r) => r.mac)) : new Set())}
                    />
                  </th>
                  <th>Address</th>
                  <th>Guest</th>
                  <th>MAC</th>
                  <th>Bridge</th>
                  <th>Gateway</th>
                  <th>State</th>
                  <th>VM</th>
                  <th>Expires</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((l) => {
                  const mismatch = l.pool_bridge && l.nic_bridge && l.pool_bridge !== l.nic_bridge
                  return (
                    <tr key={l.mac}>
                      <td>
                        <input type="checkbox" className="form-check-input" checked={selected.has(l.mac)} onChange={() => toggle(l.mac)} />
                      </td>
                      <td>
                        <Copy text={l.ip} className="font-monospace fw-semibold" />
                      </td>
                      <td>
                        {l.vmid != null ? (
                          <div className="d-flex align-items-center gap-2">
                            <span className="font-monospace fw-semibold">{l.vmid}</span>
                            <span className="text-truncate" style={{ maxWidth: 180 }}>
                              {l.guest_name || l.hostname}
                            </span>
                            {!l.guest_name && <span className="badge text-bg-warning">gone</span>}
                            {l.guest_status && l.guest_status !== 'running' && <StatusPill status={l.guest_status} />}
                          </div>
                        ) : (
                          <span className="text-body-secondary">
                            <i className="bi bi-person-gear me-1" />
                            {l.hostname || 'manual'}
                          </span>
                        )}
                      </td>
                      <td>
                        <Copy text={l.mac} className="font-monospace small text-body-secondary" />
                        {l.nic_key && <span className="badge text-bg-secondary ms-2">{l.nic_key}</span>}
                      </td>
                      <td>
                        <span className="font-monospace small">{bridgeOf(l)}</span>
                        {mismatch && (
                          <span className="badge text-bg-warning ms-1" title={`pool: ${l.pool_bridge}`}>
                            <i className="bi bi-exclamation-triangle" />
                          </span>
                        )}
                        {!l.pool_bridge && (
                          <span className="badge text-bg-secondary ms-1" title="not in pool">
                            ext
                          </span>
                        )}
                      </td>
                      <td className="font-monospace small text-body-secondary">
                        {l.gateway}/{l.cidr}
                      </td>
                      <td>
                        <StatusPill status={l.state} />
                      </td>
                      <td>
                        <LeaseHealth lease={l} check={checks[l.mac]} onCheck={() => checkOne(l)} />
                      </td>
                      <td className="small text-body-secondary" title={fmtTime(l.lease_end)}>
                        {fmtRelative(l.lease_end)}
                      </td>
                      <td className="text-end text-nowrap">
                        {l.vmid != null && (
                          <button
                            className="btn btn-sm btn-outline-secondary"
                            title="Change"
                            onClick={() => navigate('assign', { vmid: l.vmid!, mac: l.mac })}
                          >
                            <i className="bi bi-arrow-left-right" />
                          </button>
                        )}
                        <button
                          className={`btn btn-sm ${l.state === 'active' ? 'btn-outline-secondary' : 'btn-outline-success'}`}
                          title={l.state === 'active' ? 'Renew lease' : 'Reactivate lease'}
                          onClick={() => renewMany([l])}
                        >
                          <i className="bi bi-arrow-clockwise" />
                        </button>
                        <button className="btn btn-sm btn-outline-danger" title="Release" onClick={() => removeMany([l])}>
                          <i className="bi bi-trash3" />
                        </button>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
            {rows.length === 0 && <Empty icon="hdd-network">{search || bridge !== 'all' ? 'No matches' : 'No leases'}</Empty>}
          </div>
        )}
      </div>

      {selected.size > 0 && (
        <div className="card border-primary position-sticky bottom-0 mt-3 mx-auto" style={{ width: 'fit-content' }}>
          <div className="card-body py-2 d-flex align-items-center gap-2">
          <span className="fw-semibold">{selected.size} selected</span>
          <button className="btn btn-sm btn-outline-secondary" onClick={() => setSelected(new Set())}>
            Clear
          </button>
          <button className="btn btn-sm btn-success" onClick={() => renewMany((data || []).filter((l) => selected.has(l.mac)))}>
            <i className="bi bi-arrow-clockwise" /> Renew
          </button>
          <button className="btn btn-sm btn-danger" onClick={() => removeMany((data || []).filter((l) => selected.has(l.mac)))}>
            <i className="bi bi-trash3" /> Release
          </button>
          </div>
        </div>
      )}

      {manual && (
        <ManualLeaseModal
          onClose={() => setManual(false)}
          onSaved={() => {
            setManual(false)
            refreshAll()
          }}
        />
      )}
    </>
  )
}

/**
 * Two signals:
 *  - DHCP: did the VM request exactly this IP and get an ACK since the lease was (re)assigned?
 *  - Network: does the IP answer from this host, and is the ARP MAC the VM's own NIC?
 */
function LeaseHealth({ lease: l, check, onCheck }: { lease: LeaseView; check?: LeaseCheck | 'pending'; onCheck: () => void }) {
  const acked = l.acked_ip === l.ip && l.last_ack_at != null && l.last_ack_at >= l.lease_start
  const offeredOnly = !acked && l.last_offer_at != null && l.last_offer_at >= l.lease_start

  const dhcp = acked ? (
    <span className="badge text-bg-success" title={`DHCP ACK sent ${fmtTime(l.last_ack_at!)}`}>
      <i className="bi bi-check2-circle me-1" />
      took IP · {fmtRelative(l.last_ack_at!)}
    </span>
  ) : offeredOnly ? (
    <span className="badge text-bg-warning" title={`Offered ${fmtTime(l.last_offer_at!)}, but the VM never requested it`}>
      <i className="bi bi-hourglass-split me-1" />
      offered, not taken
    </span>
  ) : (
    <span
      className="badge border text-body-secondary"
      title={l.last_ack_at ? `Last ACK ${fmtTime(l.last_ack_at)}${l.acked_ip && l.acked_ip !== l.ip ? ` for ${l.acked_ip}` : ''}` : 'No DHCP request seen yet'}
    >
      <i className="bi bi-clock me-1" />
      waiting for VM
    </span>
  )

  let net: ReactNode
  if (check === 'pending') {
    net = <span className="spinner-border spinner-border-sm text-body-secondary" />
  } else if (check) {
    if (check.mac_matches === false) {
      net = (
        <span className="badge text-bg-danger" title={`ARP says ${check.ip} is at ${check.neighbor_mac}, not ${l.mac}`}>
          <i className="bi bi-exclamation-octagon me-1" />
          other MAC!
        </span>
      )
    } else if (check.ping_ok || check.mac_matches) {
      const parts = [check.ping_ok ? `ping ${check.ping_ms != null ? check.ping_ms.toFixed(1) + ' ms' : 'ok'}` : 'no ping', check.mac_matches ? 'ARP MAC matches' : null]
      net = (
        <span className="badge text-bg-success" title={parts.filter(Boolean).join(' · ')}>
          <i className="bi bi-broadcast me-1" />
          in use{check.mac_matches ? ' ✓MAC' : ''}
        </span>
      )
    } else {
      net = (
        <span className="badge text-bg-secondary" title="No ping reply and no ARP entry from this host (the VM may block ICMP, or the IP isn't up)">
          <i className="bi bi-wifi-off me-1" />
          not responding
        </span>
      )
    }
  }

  return (
    <div className="d-flex align-items-center gap-1 flex-wrap">
      {dhcp}
      {net}
      <button
        className="btn btn-sm btn-link p-0 text-body-secondary"
        title="Check if the VM is using this IP (ping + ARP)"
        onClick={onCheck}
        disabled={check === 'pending'}
      >
        <i className="bi bi-search" />
      </button>
    </div>
  )
}

interface ManualForm {
  mac: string
  ip: string
  hostname: string
  gateway: string
  cidr: string
  dns: string
}

function ManualLeaseModal({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const toast = useToast()
  const [form, setForm] = useState<ManualForm>({ mac: '', ip: '', hostname: '', gateway: '', cidr: '', dns: '' })
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)
  const set = (k: keyof ManualForm) => (e: ChangeEvent<HTMLInputElement>) => setForm({ ...form, [k]: e.target.value })

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const body: CreateLeaseRequest = { mac: form.mac, ip: form.ip, hostname: form.hostname }
      if (form.gateway) body.gateway = form.gateway
      if (form.cidr) body.cidr = Number(form.cidr)
      if (form.dns) body.dns_servers = splitList(form.dns)
      const lease = await api.post<DhcpLease>('/admin/leases', body)
      toast(
        <>
          <b className="font-monospace">{lease.ip}</b> → <span className="font-monospace">{lease.mac}</span>
        </>,
      )
      onSaved()
    } catch (err) {
      setError(err)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal title="Manual lease" onClose={onClose}>
      <form onSubmit={save}>
        <div className="row g-3">
          <div className="col-6">
            <label className="form-label">MAC</label>
            <input className="form-control font-monospace" value={form.mac} onChange={set('mac')} placeholder="bc:24:11:…" required autoFocus />
          </div>
          <div className="col-6">
            <label className="form-label">IP</label>
            <input className="form-control font-monospace" value={form.ip} onChange={set('ip')} required />
          </div>
          <div className="col-12">
            <label className="form-label">Hostname</label>
            <input className="form-control" value={form.hostname} onChange={set('hostname')} />
          </div>
          <div className="col-5">
            <label className="form-label">Gateway</label>
            <input className="form-control font-monospace" value={form.gateway} onChange={set('gateway')} placeholder="from pool" />
          </div>
          <div className="col-3">
            <label className="form-label">CIDR</label>
            <input className="form-control font-monospace" type="number" min="1" max="32" value={form.cidr} onChange={set('cidr')} placeholder="—" />
          </div>
          <div className="col-4">
            <label className="form-label">DNS</label>
            <input className="form-control font-monospace" value={form.dns} onChange={set('dns')} placeholder="1.1.1.1" />
          </div>
        </div>
        <div className="mt-3">
          <ErrorBox error={error} />
        </div>
        <div className="d-flex justify-content-end gap-2 mt-3">
          <button type="button" className="btn btn-outline-secondary" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={busy}>
            Save
          </button>
        </div>
      </form>
    </Modal>
  )
}
