import { useMemo, useState, type ChangeEvent, type FormEvent } from 'react'
import { api, downloadCsv, enc, errorMessage, ipToInt, refreshAll, splitList } from '../api.ts'
import { Copy, Empty, ErrorBox, Meter, Modal, PageHead, SearchInput, Seg, Skeleton, useConfirm, useLoader, useToast } from '../components.tsx'
import { getPref, setPref } from '../prefs.ts'
import { navigate } from '../router.ts'
import type { AddPoolRequest, AddPoolResponse, BridgeSummary, PageProps, PoolEntry, PoolInput } from '../types.ts'

type View = 'map' | 'table'
type StateFilter = 'all' | 'free' | 'used'

export default function Pool({ params }: PageProps) {
  const toast = useToast()
  const confirm = useConfirm()
  const pool = useLoader(() => api.get<PoolEntry[]>('/admin/pool'))
  const bridges = useLoader(() => api.get<BridgeSummary[]>('/admin/bridges'))
  const [bridge, setBridge] = useState(params.bridge || 'all')
  const [view, setViewState] = useState<View>(getPref('poolView', 'map') === 'table' ? 'table' : 'map')
  const [state, setState] = useState<StateFilter>('all')
  const [q, setQ] = useState(params.q || '')
  const [adding, setAdding] = useState<{ bridge: string } | null>(params.add ? { bridge: params.bridge || '' } : null)
  const [editing, setEditing] = useState<PoolEntry | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())

  const setView = (v: View) => {
    setViewState(v)
    setPref('poolView', v)
  }

  const all = pool.data || []
  const tabs = useMemo(() => {
    const names = new Set(all.map((p) => p.bridge))
    ;(bridges.data || []).forEach((b) => names.add(b.name))
    return [...names].sort()
  }, [all, bridges.data])

  const inBridge = all.filter((p) => bridge === 'all' || p.bridge === bridge)
  const rows = inBridge
    .filter((p) => state === 'all' || (state === 'free' ? !p.lease : !!p.lease))
    .filter((p) => {
      const s = q.trim().toLowerCase()
      return (
        !s ||
        p.ip.includes(s) ||
        p.notes?.toLowerCase().includes(s) ||
        p.lease?.mac.includes(s) ||
        String(p.lease?.vmid ?? '').includes(s) ||
        p.lease?.guest_name?.toLowerCase().includes(s)
      )
    })
    .sort((a, b) => a.bridge.localeCompare(b.bridge) || ipToInt(a.ip) - ipToInt(b.ip))

  const used = inBridge.filter((p) => p.lease).length
  const summary = bridge !== 'all' ? (bridges.data || []).find((b) => b.name === bridge) : null
  const sample = inBridge[0]

  const remove = async (entries: PoolEntry[]) => {
    const leased = entries.filter((e) => e.lease).length
    const ok = await confirm(
      <>
        Remove {entries.length === 1 ? <b className="font-monospace">{entries[0].ip}</b> : <b>{entries.length} addresses</b>}?
        {leased > 0 && (
          <div className="alert alert-warning mt-3 mb-0 py-2">
            <i className="bi bi-exclamation-triangle me-2" />
            {leased} leased
          </div>
        )}
      </>,
      { title: 'Remove from pool', confirmLabel: 'Remove' },
    )
    if (!ok) return
    try {
      for (const e of entries) await api.del(`/admin/pool/${enc(e.ip)}?force=true`)
      toast(`Removed ${entries.length}`)
    } catch (e) {
      toast(errorMessage(e), 'danger')
    }
    setSelected(new Set())
    refreshAll()
  }

  const toggle = (ip: string) => {
    const next = new Set(selected)
    if (next.has(ip)) next.delete(ip)
    else next.add(ip)
    setSelected(next)
  }

  const exportCsv = () =>
    downloadCsv('feathertail-pool.csv', [
      ['ip', 'bridge', 'gateway', 'cidr', 'dns', 'server_ip', 'notes', 'leased_to', 'vmid'],
      ...rows.map((p) => [p.ip, p.bridge, p.gateway, p.cidr, p.dns_servers.join(' '), p.server_ip, p.notes, p.lease?.mac, p.lease?.vmid]),
    ])

  return (
    <>
      <PageHead
        title="Address pool"
        meta={
          <>
            <span className="badge text-bg-success">
                            {inBridge.length - used} free
            </span>
            <span className="badge text-bg-primary">{used} leased</span>
            {sample && bridge !== 'all' && (
              <span className="badge border text-body font-monospace">
                gw {sample.gateway}/{sample.cidr}
              </span>
            )}
            {summary && summary.host_addresses.length > 0 && (
              <span className="badge border text-body font-monospace">host {summary.host_addresses.join(', ')}</span>
            )}
          </>
        }
      >
        <button className="btn btn-outline-secondary" onClick={exportCsv} disabled={!rows.length}>
          <i className="bi bi-download" /> CSV
        </button>
        <button className="btn btn-primary" onClick={() => setAdding({ bridge: bridge === 'all' ? '' : bridge })}>
          <i className="bi bi-plus-lg" /> Add range
        </button>
      </PageHead>
      <ErrorBox error={pool.error} onRetry={pool.reload} />

      <div className="row g-3 mb-3">
        {tabs.map((b) => {
          const entries = all.filter((p) => p.bridge === b)
          const u = entries.filter((p) => p.lease).length
          return (
            <div className="col-6 col-md-4 col-xl-3" key={b}>
              <div
                className={`card card-body cursor-pointer ${bridge === b ? 'border-primary' : ''}`}
                onClick={() => setBridge(bridge === b ? 'all' : b)}
              >
                <div className="d-flex justify-content-between mb-2">
                  <span className="font-monospace fw-semibold">{b}</span>
                  <span className="small">
                    {u}/{entries.length}
                  </span>
                </div>
                <Meter value={u} max={entries.length} />
              </div>
            </div>
          )
        })}
      </div>

      <div className="card">
        <div className="card-header d-flex align-items-center justify-content-between gap-2 flex-wrap">
          <SearchInput value={q} onChange={setQ} placeholder="IP, VMID, MAC, note" />
          <div className="d-flex gap-2 flex-wrap">
            <Seg
              value={state}
              onChange={setState}
              options={[
                { value: 'all', label: 'All', count: inBridge.length },
                { value: 'free', label: 'Free', count: inBridge.length - used },
                { value: 'used', label: 'Leased', count: used },
              ]}
            />
            <Seg
              value={view}
              onChange={setView}
              options={[
                { value: 'map', label: '', icon: 'grid-3x3-gap' },
                { value: 'table', label: '', icon: 'list-ul' },
              ]}
            />
          </div>
        </div>

        {!pool.data ? (
          <div className="p-3">
            <Skeleton rows={1} />
          </div>
        ) : rows.length === 0 ? (
          <Empty icon="diagram-3">
            <div className="mb-3">{all.length ? 'No matches' : 'Empty pool'}</div>
            {!all.length && (
              <button className="btn btn-primary" onClick={() => setAdding({ bridge: '' })}>
                <i className="bi bi-plus-lg" /> Add range
              </button>
            )}
          </Empty>
        ) : view === 'map' ? (
          <div className="card-body">
            {[...new Set(rows.map((r) => r.bridge))].map((b) => {
              const group = rows.filter((r) => r.bridge === b)
              const total = all.filter((p) => p.bridge === b)
              const u = total.filter((p) => p.lease).length
              return (
                <div className="mb-4" key={b}>
                  <div className="d-flex align-items-center gap-2 mb-2 small">
                    <span className="font-monospace fw-bold">{b}</span>
                    <span className="font-monospace text-body-secondary">
                      gw {group[0].gateway}/{group[0].cidr}
                    </span>
                    <span className="ms-auto">
                      {u}/{total.length}
                    </span>
                    <div style={{ width: 120 }}>
                      <Meter value={u} max={total.length} />
                    </div>
                  </div>
                  <div className="ip-grid">
                    {group.map((p) => (
                      <button key={p.ip} className={`btn btn-sm btn-outline-${p.lease ? 'primary' : 'success'}`} onClick={() => setEditing(p)} title={p.notes || ''}>
                        <div className="font-monospace fw-semibold">{p.ip}</div>
                        <div className="sub">
                          {p.lease ? `${p.lease.vmid ?? ''} ${p.lease.guest_name || p.lease.hostname || p.lease.mac}` : p.notes || 'free'}
                        </div>
                      </button>
                    ))}
                  </div>
                </div>
              )
            })}
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
                      checked={rows.length > 0 && rows.every((r) => selected.has(r.ip))}
                      onChange={(e) => setSelected(e.target.checked ? new Set(rows.map((r) => r.ip)) : new Set())}
                    />
                  </th>
                  <th>Address</th>
                  <th>Bridge</th>
                  <th>Gateway</th>
                  <th>DNS</th>
                  <th>Holder</th>
                  <th>Note</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((p) => (
                  <tr key={p.ip}>
                    <td>
                      <input type="checkbox" className="form-check-input" checked={selected.has(p.ip)} onChange={() => toggle(p.ip)} />
                    </td>
                    <td>
                      <Copy text={p.ip} className="font-monospace fw-semibold" />
                    </td>
                    <td className="font-monospace small">{p.bridge}</td>
                    <td className="font-monospace small text-body-secondary">
                      {p.gateway}/{p.cidr}
                      {p.server_ip && <div>srv {p.server_ip}</div>}
                    </td>
                    <td className="font-monospace small text-body-secondary">{p.dns_servers.join(', ') || '—'}</td>
                    <td>
                      {p.lease ? (
                        <div>
                          <span className="font-monospace fw-semibold">{p.lease.vmid ?? '—'}</span> {p.lease.guest_name || p.lease.hostname}
                          <div className="small text-body-secondary font-monospace">{p.lease.mac}</div>
                        </div>
                      ) : (
                        <span className="badge text-bg-success">
                                                    free
                        </span>
                      )}
                    </td>
                    <td className="small text-body-secondary">{p.notes}</td>
                    <td className="text-end text-nowrap">
                      {!p.lease && (
                        <button className="btn btn-sm btn-outline-secondary" title="Assign" onClick={() => navigate('assign')}>
                          <i className="bi bi-plus-square" />
                        </button>
                      )}
                      <button className="btn btn-sm btn-outline-secondary" title="Edit" onClick={() => setEditing(p)}>
                        <i className="bi bi-pencil" />
                      </button>
                      <button className="btn btn-sm btn-outline-danger" title="Remove" onClick={() => remove([p])}>
                        <i className="bi bi-trash3" />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {selected.size > 0 && view === 'table' && (
        <div className="card border-primary position-sticky bottom-0 mt-3 mx-auto" style={{ width: 'fit-content' }}>
          <div className="card-body py-2 d-flex align-items-center gap-2">
          <span className="fw-semibold">{selected.size} selected</span>
          <button className="btn btn-sm btn-outline-secondary" onClick={() => setSelected(new Set())}>
            Clear
          </button>
          <button className="btn btn-sm btn-danger" onClick={() => remove(all.filter((r) => selected.has(r.ip)))}>
            <i className="bi bi-trash3" /> Remove
          </button>
          </div>
        </div>
      )}

      {adding && (
        <AddIpsModal
          initial={adding}
          bridges={bridges.data || []}
          pool={all}
          onClose={() => setAdding(null)}
          onSaved={(b) => {
            setAdding(null)
            if (b) setBridge(b)
            refreshAll()
          }}
        />
      )}
      {editing && (
        <EditIpModal
          entry={editing}
          bridges={bridges.data || []}
          onClose={() => setEditing(null)}
          onRemove={() => {
            const e = editing
            setEditing(null)
            remove([e])
          }}
          onSaved={() => {
            setEditing(null)
            refreshAll()
          }}
        />
      )}
    </>
  )
}

// ---------------------------------------------------------------- range preview (mirrors dhcp::expand_ip_list)
function parseIp(s: string): number | null {
  const parts = s.trim().split('.')
  if (parts.length !== 4 || parts.some((p) => !/^\d{1,3}$/.test(p) || Number(p) > 255)) return null
  return parts.reduce((acc, p) => acc * 256 + Number(p), 0)
}

function countIps(input: string): { count: number; bad: string[] } {
  let count = 0
  const bad: string[] = []
  for (const raw of input.split(/[,;\n]/)) {
    const item = raw.trim()
    if (!item) continue
    const [a, b] = item.split('-').map((s) => s.trim())
    const start = parseIp(a)
    if (start == null) {
      bad.push(item)
      continue
    }
    if (b === undefined) {
      count++
      continue
    }
    const end = /^\d{1,3}$/.test(b) ? Math.floor(start / 256) * 256 + Number(b) : parseIp(b)
    if (end == null || end < start || Number(b) > 255) bad.push(item)
    else count += end - start + 1
  }
  return { count, bad }
}

// ---------------------------------------------------------------- modals
interface NetForm {
  bridge: string
  gateway: string
  cidr: string
  dns: string
  server_ip: string
  notes: string
}

function toPoolInput(form: NetForm): PoolInput {
  return {
    bridge: form.bridge.trim(),
    gateway: form.gateway.trim(),
    cidr: Number(form.cidr),
    dns_servers: splitList(form.dns),
    server_ip: form.server_ip.trim() || null,
    notes: form.notes.trim() || null,
  }
}

type InputChange = ChangeEvent<HTMLInputElement | HTMLTextAreaElement>

function BridgePicker({ value, onChange, bridges }: { value: string; onChange: (b: string) => void; bridges: BridgeSummary[] }) {
  return (
    <>
      <input className="form-control font-monospace" list="ft-bridges" value={value} onChange={(e) => onChange(e.target.value)} placeholder="vmbr1" required />
      <datalist id="ft-bridges">
        {bridges.map((b) => (
          <option key={b.name} value={b.name}>
            {b.host_addresses.join(', ')}
          </option>
        ))}
      </datalist>
      {bridges.length > 0 && (
        <div className="d-flex flex-wrap gap-1 mt-2">
          {bridges.map((b) => (
            <button
              key={b.name}
              type="button"
              className={`badge ${b.name === value ? 'text-bg-primary' : 'border text-body'} font-monospace border`}
              style={{ cursor: 'pointer', background: b.name === value ? undefined : 'transparent' }}
              onClick={() => onChange(b.name)}
            >
              {b.name}
            </button>
          ))}
        </div>
      )}
    </>
  )
}

function NetFields({ form, set }: { form: NetForm; set: (k: keyof NetForm) => (e: InputChange) => void }) {
  return (
    <>
      <div className="col-md-5">
        <label className="form-label">Gateway</label>
        <input className="form-control font-monospace" value={form.gateway} onChange={set('gateway')} placeholder="10.0.0.1" required />
      </div>
      <div className="col-md-2">
        <label className="form-label">CIDR</label>
        <input className="form-control font-monospace" type="number" min="1" max="32" value={form.cidr} onChange={set('cidr')} required />
      </div>
      <div className="col-md-5">
        <label className="form-label">DNS</label>
        <input className="form-control font-monospace" value={form.dns} onChange={set('dns')} placeholder="1.1.1.1, 8.8.8.8" />
      </div>
      <div className="col-md-5">
        <label className="form-label">Server ID</label>
        <input className="form-control font-monospace" value={form.server_ip} onChange={set('server_ip')} placeholder="gateway" />
      </div>
      <div className="col-md-7">
        <label className="form-label">Note</label>
        <input className="form-control" value={form.notes} onChange={set('notes')} />
      </div>
    </>
  )
}

interface AddIpsModalProps {
  initial: { bridge: string }
  bridges: BridgeSummary[]
  pool: PoolEntry[]
  onClose: () => void
  onSaved: (bridge: string) => void
}

function AddIpsModal({ initial, bridges, pool, onClose, onSaved }: AddIpsModalProps) {
  const toast = useToast()
  const template = (b: string) => pool.find((p) => p.bridge === b)
  const t = template(initial.bridge)
  const [form, setForm] = useState<NetForm & { ips: string }>({
    bridge: initial.bridge || '',
    ips: '',
    gateway: t?.gateway || '',
    cidr: String(t?.cidr ?? 24),
    dns: t?.dns_servers.join(', ') || '',
    server_ip: t?.server_ip || '',
    notes: '',
  })
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)
  const set = (k: keyof typeof form) => (e: InputChange) => setForm({ ...form, [k]: e.target.value })
  const preview = countIps(form.ips)

  const setBridge = (b: string) => {
    const tpl = template(b)
    setForm((f) => ({
      ...f,
      bridge: b,
      ...(tpl ? { gateway: tpl.gateway, cidr: String(tpl.cidr), dns: tpl.dns_servers.join(', '), server_ip: tpl.server_ip || '' } : {}),
    }))
  }

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const body: AddPoolRequest = { ...toPoolInput(form), ips: form.ips }
      const res = await api.post<AddPoolResponse>('/admin/pool', body)
      toast(
        <>
          +{res.added} on <span className="font-monospace">{body.bridge}</span>
          {res.skipped.length > 0 && <span className="text-body-secondary"> · skipped {res.skipped.join(', ')}</span>}
        </>,
      )
      if (res.moved_from_other_bridge.length) toast(`Moved: ${res.moved_from_other_bridge.join(', ')}`, 'warning')
      onSaved(body.bridge)
    } catch (err) {
      setError(err)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal title="Add addresses" onClose={onClose} size="modal-lg">
      <form onSubmit={save}>
        <div className="row g-3">
          <div className="col-md-5">
            <label className="form-label">Bridge</label>
            <BridgePicker value={form.bridge} onChange={setBridge} bridges={bridges} />
          </div>
          <div className="col-md-7">
            <label className="form-label d-flex justify-content-between">
              <span>Addresses</span>
              {form.ips.trim() && (
                <span className={preview.bad.length ? 'text-danger' : 'text-success'}>
                  {preview.bad.length ? `invalid: ${preview.bad[0]}` : `${preview.count} IPs`}
                </span>
              )}
            </label>
            <textarea
              className="form-control font-monospace"
              rows={3}
              value={form.ips}
              onChange={set('ips')}
              placeholder={'10.0.0.40-100\n10.0.0.120, 10.0.0.121'}
              required
              autoFocus
            />
          </div>
          <NetFields form={form} set={set} />
        </div>
        <div className="mt-3">
          <ErrorBox error={error} />
        </div>
        <div className="d-flex justify-content-end gap-2 mt-3">
          <button type="button" className="btn btn-outline-secondary" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={busy || preview.bad.length > 0 || preview.count === 0}>
            {busy && <span className="spinner-border spinner-border-sm" />}
            Add {preview.count > 0 && preview.count}
          </button>
        </div>
      </form>
    </Modal>
  )
}

interface EditIpModalProps {
  entry: PoolEntry
  bridges: BridgeSummary[]
  onClose: () => void
  onSaved: () => void
  onRemove: () => void
}

function EditIpModal({ entry, bridges, onClose, onSaved, onRemove }: EditIpModalProps) {
  const toast = useToast()
  const [form, setForm] = useState<NetForm>({
    bridge: entry.bridge,
    gateway: entry.gateway,
    cidr: String(entry.cidr),
    dns: entry.dns_servers.join(', '),
    server_ip: entry.server_ip || '',
    notes: entry.notes || '',
  })
  const [error, setError] = useState<unknown>(null)
  const set = (k: keyof NetForm) => (e: InputChange) => setForm({ ...form, [k]: e.target.value })

  const save = async (e: FormEvent) => {
    e.preventDefault()
    try {
      await api.put(`/admin/pool/${enc(entry.ip)}`, { ...toPoolInput(form), ip: entry.ip })
      toast(<><span className="font-monospace">{entry.ip}</span> saved</>)
      onSaved()
    } catch (err) {
      setError(err)
    }
  }

  return (
    <Modal title={<span className="font-monospace">{entry.ip}</span>} onClose={onClose} size="modal-lg">
      <form onSubmit={save}>
        {entry.lease ? (
          <div className="alert alert-info py-2 d-flex align-items-center gap-2">
            <i className="bi bi-link-45deg" />
            <span className="font-monospace">{entry.lease.vmid ?? '—'}</span> {entry.lease.guest_name || entry.lease.hostname}
            <span className="font-monospace text-body-secondary ms-auto">{entry.lease.mac}</span>
          </div>
        ) : (
          <div className="d-flex align-items-center gap-2 mb-3">
            <span className="badge text-bg-success">
                            free
            </span>
            <button type="button" className="btn btn-sm btn-outline-secondary ms-auto" onClick={() => navigate('assign')}>
              <i className="bi bi-plus-square" /> Assign
            </button>
          </div>
        )}
        <div className="row g-3">
          <div className="col-12">
            <label className="form-label">Bridge</label>
            <BridgePicker value={form.bridge} onChange={(b) => setForm({ ...form, bridge: b })} bridges={bridges} />
          </div>
          <NetFields form={form} set={set} />
        </div>
        <div className="mt-3">
          <ErrorBox error={error} />
        </div>
        <div className="d-flex gap-2 mt-3">
          <button type="button" className="btn btn-outline-danger" onClick={onRemove}>
            <i className="bi bi-trash3" /> Remove
          </button>
          <button type="button" className="btn btn-outline-secondary ms-auto" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary">Save</button>
        </div>
      </form>
    </Modal>
  )
}
