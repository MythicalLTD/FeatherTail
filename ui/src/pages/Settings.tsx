import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from 'react'
import { api, errorMessage } from '../api.ts'
import { Copy, ErrorBox, PageHead, SearchInput, Seg, Skeleton, useConfirm, useLoader, useToast } from '../components.tsx'
import type { LogsResponse, SettingsView, UpdateDhcpSettings } from '../types.ts'

type Tab = 'dhcp' | 'security' | 'logs'

export default function Settings() {
  const [tab, setTab] = useState<Tab>('dhcp')
  return (
    <>
      <PageHead title="Settings">
        <Seg
          value={tab}
          onChange={setTab}
          options={[
            { value: 'dhcp', label: 'DHCP', icon: 'broadcast' },
            { value: 'security', label: 'Access', icon: 'shield-lock' },
            { value: 'logs', label: 'Logs', icon: 'terminal' },
          ]}
        />
      </PageHead>
      {tab === 'dhcp' && <DhcpSettings />}
      {tab === 'security' && <PasswordCard />}
      {tab === 'logs' && <Logs />}
    </>
  )
}

interface DhcpForm {
  enabled: boolean
  server_ip: string
  lease_time_secs: string
}

const LEASE_PRESETS: [string, number][] = [
  ['1h', 3600],
  ['12h', 43200],
  ['1d', 86400],
  ['7d', 604800],
]

function DhcpSettings() {
  const toast = useToast()
  const confirm = useConfirm()
  const { data, error, reload } = useLoader(() => api.get<SettingsView>('/admin/settings'))
  const [form, setForm] = useState<DhcpForm | null>(null)
  const [dirty, setDirty] = useState(false)
  const [pendingRestart, setPendingRestart] = useState(false)

  useEffect(() => {
    if (data && !dirty) {
      setForm({ enabled: data.dhcp_enabled, server_ip: data.dhcp_server_ip, lease_time_secs: String(data.dhcp_lease_time_secs) })
      setPendingRestart(data.dhcp_enabled !== data.running_dhcp_enabled)
    }
  }, [data, dirty])

  if (!data || !form) return error ? <ErrorBox error={error} onRetry={reload} /> : <Skeleton rows={1} />

  const set = <K extends keyof DhcpForm>(k: K, v: DhcpForm[K]) => {
    setForm({ ...form, [k]: v })
    setDirty(true)
  }

  const save = async (e: FormEvent) => {
    e.preventDefault()
    try {
      const body: UpdateDhcpSettings = { ...form, lease_time_secs: Number(form.lease_time_secs) }
      await api.put('/admin/settings', body)
      toast('Saved')
      setDirty(false)
      setPendingRestart(true)
    } catch (err) {
      toast(errorMessage(err), 'danger')
    }
  }

  const restart = async () => {
    const ok = await confirm('Restart FeatherTail now?', { title: 'Restart service', confirmLabel: 'Restart', variant: 'warning' })
    if (!ok) return
    try {
      await api.post('/admin/restart')
    } catch {
      /* connection drops during restart */
    }
    toast('Restarting…', 'info')
    setTimeout(() => window.location.reload(), 4000)
  }

  const secs = Number(form.lease_time_secs) || 0

  return (
    <div className="row g-3">
      <div className="col-lg-8">
        <form className="card" onSubmit={save}>
          <div className="card-header d-flex align-items-center justify-content-between gap-2">
            <span className="d-flex align-items-center gap-2">
              <i className="bi bi-broadcast" /> Server
            </span>
            <div className="form-check form-switch m-0">
              <input
                className="form-check-input"
                type="checkbox"
                role="switch"
                id="dhcp-enabled"
                checked={form.enabled}
                onChange={(e) => set('enabled', e.target.checked)}
              />
              <label className="form-check-label fw-normal" htmlFor="dhcp-enabled">
                {form.enabled ? 'Enabled' : 'Disabled'}
              </label>
            </div>
          </div>
          <div className="card-body">
            <div className="row g-3">
              <div className="col-md-6">
                <label className="form-label">Default server ID</label>
                <input className="form-control font-monospace" value={form.server_ip} onChange={(e) => set('server_ip', e.target.value)} />
              </div>
              <div className="col-md-6">
                <label className="form-label d-flex justify-content-between">
                  <span>Lease time</span>
                  <span className="text-body-secondary">{humanSecs(secs)}</span>
                </label>
                <div className="input-group">
                  <input
                    className="form-control font-monospace"
                    type="number"
                    min="60"
                    value={form.lease_time_secs}
                    onChange={(e) => set('lease_time_secs', e.target.value)}
                  />
                  <span className="input-group-text">s</span>
                </div>
                <div className="d-flex gap-1 mt-2">
                  {LEASE_PRESETS.map(([label, v]) => (
                    <button
                      key={label}
                      type="button"
                      className={`badge ${secs === v ? 'text-bg-primary' : 'border text-body'} border`}
                      style={{ cursor: 'pointer', background: secs === v ? undefined : 'transparent' }}
                      onClick={() => set('lease_time_secs', String(v))}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </div>
            </div>
            {pendingRestart && (
              <div className="alert alert-warning d-flex align-items-center gap-2 mt-3 mb-0 py-2">
                <i className="bi bi-arrow-repeat" />
                <span className="me-auto">Restart required</span>
                <button type="button" className="btn btn-sm btn-warning" onClick={restart}>
                  Restart now
                </button>
              </div>
            )}
          </div>
          <div className="card-footer bg-transparent d-flex gap-2 p-3">
            <button type="button" className="btn btn-outline-secondary" onClick={restart}>
              <i className="bi bi-arrow-repeat" /> Restart service
            </button>
            <button className="btn btn-primary ms-auto" disabled={!dirty}>
              Save
            </button>
          </div>
        </form>
      </div>
      <div className="col-lg-4">
        <div className="card h-100">
          <div className="card-header d-flex align-items-center justify-content-between gap-2">
            <span className="d-flex align-items-center gap-2">
              <i className="bi bi-info-circle" /> Runtime
            </span>
          </div>
          <div className="card-body">
            <Kv k="Listener" v={<span className={`badge ${data.running_dhcp_enabled ? 'text-bg-success' : 'text-bg-secondary'}`}><span className="dot" />{data.running_dhcp_enabled ? 'running' : 'off'}</span>} />
            <Kv k="DHCP bind" v={<span className="font-monospace">{data.dhcp_bind}</span>} />
            <Kv k="API bind" v={<span className="font-monospace">{data.api_bind}</span>} />
            <Kv k="Config" v={<Copy text={data.config_path} className="font-monospace small" />} />
            <Kv k="Password" v={data.password_configured ? <span className="badge text-bg-success">set</span> : <span className="badge text-bg-secondary">token only</span>} />
          </div>
        </div>
      </div>
    </div>
  )
}

function Kv({ k, v }: { k: string; v: ReactNode }) {
  return (
    <div className="d-flex justify-content-between align-items-center py-2 border-bottom gap-3">
      <span className="text-body-secondary small">{k}</span>
      <span className="text-end text-truncate">{v}</span>
    </div>
  )
}

function humanSecs(s: number): string {
  if (s >= 86400 && s % 86400 === 0) return `${s / 86400}d`
  if (s >= 3600 && s % 3600 === 0) return `${s / 3600}h`
  if (s >= 60 && s % 60 === 0) return `${s / 60}m`
  return `${s}s`
}

function strength(pw: string): { score: number; label: string } {
  let score = 0
  if (pw.length >= 8) score++
  if (pw.length >= 14) score++
  if (/[A-Z]/.test(pw) && /[a-z]/.test(pw)) score++
  if (/\d/.test(pw)) score++
  if (/[^A-Za-z0-9]/.test(pw)) score++
  return { score, label: ['too short', 'weak', 'fair', 'good', 'strong', 'excellent'][score] }
}

function PasswordCard() {
  const toast = useToast()
  const [pw, setPw] = useState('')
  const [pw2, setPw2] = useState('')
  const [error, setError] = useState<unknown>(null)
  const s = strength(pw)

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    if (pw !== pw2) {
      setError(new Error('Passwords do not match'))
      return
    }
    try {
      await api.post('/auth/password', { new_password: pw })
      toast('Password updated')
      setTimeout(() => window.location.reload(), 1200)
    } catch (err) {
      setError(err)
    }
  }

  const tone = s.score <= 1 ? 'bg-danger' : s.score <= 2 ? 'bg-warning' : 'bg-success'

  return (
    <div className="row">
      <div className="col-lg-6">
        <form className="card" onSubmit={save}>
          <div className="card-header d-flex align-items-center justify-content-between gap-2">
            <span className="d-flex align-items-center gap-2">
              <i className="bi bi-key" /> Admin password
            </span>
          </div>
          <div className="card-body">
            <label className="form-label">New</label>
            <input type="password" className="form-control mb-2" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} />
            {pw && (
              <div className="d-flex align-items-center gap-2 mb-3">
                <div className="progress flex-grow-1" style={{ height: 6 }}>
                  <div className={`progress-bar ${tone}`} style={{ width: `${(s.score / 5) * 100}%` }} />
                </div>
                <span className="small text-body-secondary" style={{ minWidth: 70 }}>
                  {s.label}
                </span>
              </div>
            )}
            <label className="form-label">Repeat</label>
            <input
              type="password"
              className={`form-control mb-3 ${pw2 && pw2 !== pw ? 'is-invalid' : ''}`}
              autoComplete="new-password"
              value={pw2}
              onChange={(e) => setPw2(e.target.value)}
            />
            <ErrorBox error={error} />
            <div className="d-flex align-items-center">
              <span className="small text-body-secondary">
                <i className="bi bi-box-arrow-right me-1" />
                Signs out all sessions
              </span>
              <button className="btn btn-primary ms-auto" disabled={pw.length < 8 || pw !== pw2}>
                Update
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  )
}

function Logs() {
  const [lines, setLines] = useState(200)
  const [q, setQ] = useState('')
  const [level, setLevel] = useState<'all' | 'warn' | 'error'>('all')
  const { data, error, loading, reload } = useLoader(() => api.get<LogsResponse>(`/system/logs?lines=${lines}`), [lines])

  const shown = useMemo(() => {
    const all = (data?.content || '').split('\n')
    const s = q.toLowerCase()
    return all.filter((l) => {
      if (level === 'warn' && !/WARN|ERROR/.test(l)) return false
      if (level === 'error' && !/ERROR/.test(l)) return false
      return !s || l.toLowerCase().includes(s)
    })
  }, [data, q, level])

  return (
    <div className="card">
      <div className="card-header d-flex align-items-center justify-content-between gap-2 flex-wrap">
        <SearchInput value={q} onChange={setQ} placeholder="Filter" />
        <div className="d-flex gap-2">
          <Seg
            value={level}
            onChange={setLevel}
            options={[
              { value: 'all', label: 'All' },
              { value: 'warn', label: 'Warn+' },
              { value: 'error', label: 'Error' },
            ]}
          />
          <select className="form-select form-select-sm" style={{ width: 'auto' }} value={lines} onChange={(e) => setLines(Number(e.target.value))}>
            {[100, 200, 500, 1000].map((n) => (
              <option key={n} value={n}>
                {n} lines
              </option>
            ))}
          </select>
          <button className="btn btn-sm btn-outline-secondary" onClick={reload} disabled={loading}>
            <i className="bi bi-arrow-clockwise" />
          </button>
        </div>
      </div>
      <ErrorBox error={error} />
      <pre className="font-monospace small m-0 p-3 bg-body-tertiary" style={{ maxHeight: 560, overflow: 'auto', whiteSpace: 'pre-wrap', }}>
        {shown.map((l, i) => (
          <div key={i} className={/ERROR/.test(l) ? 'text-danger' : /WARN/.test(l) ? 'text-warning' : ''}>
            {l || ' '}
          </div>
        ))}
      </pre>
    </div>
  )
}
