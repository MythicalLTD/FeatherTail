import { useCallback, useEffect, useState, type ComponentType } from 'react'
import { api, refreshAll, setUnauthorizedHandler } from './api.ts'
import { ConfirmProvider, ToastProvider, useLoader } from './components.tsx'
import CommandPalette from './CommandPalette.tsx'
import { applyTheme, getLive, getTheme, setLive, type Theme } from './prefs.ts'
import { parseHash, type PageId, type Route } from './router.ts'
import type { Overview, PageProps, SessionInfo } from './types.ts'
import Login from './pages/Login.tsx'
import Status from './pages/Status.tsx'
import Assign from './pages/Assign.tsx'
import Leases from './pages/Leases.tsx'
import Pool from './pages/Pool.tsx'
import Guests from './pages/Guests.tsx'
import Settings from './pages/Settings.tsx'
import logo from './assets/featherpanel-logo.png'

export interface PageDef {
  id: PageId
  label: string
  icon: string
  group: string
  component: ComponentType<PageProps>
}

export const PAGES: PageDef[] = [
  { id: 'status', label: 'Overview', icon: 'grid-1x2', group: 'Network', component: Status },
  { id: 'assign', label: 'Assign IP', icon: 'plus-square', group: 'Network', component: Assign },
  { id: 'leases', label: 'Leases', icon: 'hdd-network', group: 'Network', component: Leases },
  { id: 'pool', label: 'Address pool', icon: 'diagram-3', group: 'Network', component: Pool },
  { id: 'guests', label: 'Guests', icon: 'pc-display', group: 'Inventory', component: Guests },
  { id: 'settings', label: 'Settings', icon: 'sliders', group: 'System', component: Settings },
]

interface AuthState {
  checked: boolean
  authenticated: boolean
  password_configured?: boolean
}

const LIVE_INTERVAL_MS = 10_000

export default function App() {
  const [auth, setAuth] = useState<AuthState>({ checked: false, authenticated: false })
  const [route, setRoute] = useState<Route>(parseHash())
  const [theme, setTheme] = useState<Theme>(getTheme())

  useEffect(() => applyTheme(theme), [theme])

  const checkSession = useCallback(async () => {
    try {
      const s = await api.get<SessionInfo>('/auth/session')
      setAuth({ checked: true, ...s })
    } catch {
      setAuth({ checked: true, authenticated: false })
    }
  }, [])

  useEffect(() => {
    setUnauthorizedHandler(() => setAuth((a) => ({ ...a, authenticated: false })))
    checkSession()
    const onHash = () => setRoute(parseHash())
    window.addEventListener('hashchange', onHash)
    return () => window.removeEventListener('hashchange', onHash)
  }, [checkSession])

  const logout = async () => {
    await api.post('/auth/logout').catch(() => {})
    setAuth({ checked: true, authenticated: false })
  }

  if (!auth.checked) return null

  return (
    <ToastProvider>
      <ConfirmProvider>
        {!auth.authenticated ? (
          <Login onLogin={checkSession} passwordConfigured={!!auth.password_configured} />
        ) : (
          <Shell
            route={route}
            onLogout={logout}
            theme={theme}
            onTheme={() => setTheme((t) => (t === 'dark' ? 'light' : 'dark'))}
          />
        )}
      </ConfirmProvider>
    </ToastProvider>
  )
}

interface ShellProps {
  route: Route
  onLogout: () => void
  theme: Theme
  onTheme: () => void
}

function Shell({ route, onLogout, theme, onTheme }: ShellProps) {
  const page = PAGES.find((p) => p.id === route.page) ?? PAGES[0]
  const Page = page.component
  const overview = useLoader(() => api.get<Overview>('/admin/overview'))
  const [live, setLiveState] = useState(getLive())
  const [palette, setPalette] = useState(false)

  useEffect(() => {
    if (!live) return
    const id = window.setInterval(() => {
      if (document.visibilityState === 'visible') refreshAll()
    }, LIVE_INTERVAL_MS)
    return () => window.clearInterval(id)
  }, [live])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPalette((p) => !p)
      } else if (e.key === '/' && !(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement)) {
        e.preventDefault()
        setPalette(true)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const o = overview.data
  const counts: Partial<Record<PageId, { n: number; alert?: boolean }>> = o
    ? {
        leases: { n: o.lease_count },
        pool: { n: o.pool_count },
        guests: o.nics_without_lease ? { n: o.nics_without_lease, alert: true } : { n: o.guest_count },
      }
    : {}
  const groups = [...new Set(PAGES.map((p) => p.group))]

  return (
    <div className="ft-shell">
      <aside className="ft-sidebar d-flex flex-column bg-body-tertiary border-end">
        <a href="#status" className="d-flex align-items-center gap-2 p-3 text-body text-decoration-none border-bottom">
          <img src={logo} alt="FeatherPanel" className="ft-logo" />
          <div className="min-w-0">
            <div className="fw-bold lh-1">FeatherTail</div>
            <small className="text-body-secondary text-truncate d-block">{o ? `${o.hostname || o.daemon} · v${o.version}` : ' '}</small>
          </div>
        </a>
        <div className="flex-grow-1 overflow-auto p-2">
          {groups.map((g) => (
            <div key={g} className="mb-2">
              <div className="small text-body-secondary text-uppercase fw-semibold px-2 pt-2 pb-1">{g}</div>
              <ul className="nav nav-pills flex-column">
                {PAGES.filter((p) => p.group === g).map((p) => {
                  const c = counts[p.id]
                  const active = p.id === route.page
                  return (
                    <li className="nav-item" key={p.id}>
                      <a href={`#${p.id}`} className={`nav-link d-flex align-items-center gap-2 ${active ? 'active' : 'link-body-emphasis'}`}>
                        <i className={`bi bi-${p.icon}`} />
                        {p.label}
                        {c && (
                          <span className={`badge ms-auto ${c.alert ? 'text-bg-warning' : active ? 'text-bg-light' : 'text-bg-secondary'}`}>{c.n}</span>
                        )}
                      </a>
                    </li>
                  )
                })}
              </ul>
            </div>
          ))}
        </div>
        <div className="border-top p-2 d-flex align-items-center gap-1">
          <span className={`badge me-auto ${o?.dhcp_listener_enabled ? 'text-bg-success' : 'text-bg-secondary'}`} title={o?.dhcp_bind}>
            DHCP {o?.dhcp_listener_enabled ? 'on' : 'off'}
          </span>
          <button className="btn btn-sm btn-outline-secondary" onClick={onTheme} title="Theme">
            <i className={`bi bi-${theme === 'dark' ? 'sun' : 'moon-stars'}`} />
          </button>
          <button className="btn btn-sm btn-outline-secondary" onClick={onLogout} title="Log out">
            <i className="bi bi-box-arrow-right" />
          </button>
        </div>
      </aside>

      <div className="ft-body">
        <nav className="navbar bg-body border-bottom px-4 sticky-top">
          <ol className="breadcrumb mb-0">
            <li className="breadcrumb-item text-body-secondary">{page.group}</li>
            <li className="breadcrumb-item active fw-semibold">{page.label}</li>
          </ol>
          <div className="d-flex align-items-center gap-2">
            <button className="btn btn-sm btn-outline-secondary d-flex align-items-center gap-2" onClick={() => setPalette(true)}>
              <i className="bi bi-search" />
              <span className="d-none d-md-inline">Search</span>
              <kbd className="d-none d-md-inline">Ctrl K</kbd>
            </button>
            <button
              className={`btn btn-sm ${live ? 'btn-success' : 'btn-outline-secondary'}`}
              onClick={() => {
                setLive(!live)
                setLiveState(!live)
              }}
              title={live ? `Auto-refresh every ${LIVE_INTERVAL_MS / 1000}s` : 'Auto-refresh paused'}
            >
              <i className={`bi bi-${live ? 'broadcast' : 'pause-fill'} me-1`} />
              {live ? 'Live' : 'Paused'}
            </button>
            <button className="btn btn-sm btn-outline-secondary" onClick={refreshAll} title="Refresh">
              <i className="bi bi-arrow-clockwise" />
            </button>
          </div>
        </nav>
        <main className="ft-main p-4">
          <Page key={route.page + JSON.stringify(route.params)} params={route.params} />
        </main>
      </div>

      {palette && <CommandPalette pages={PAGES} onClose={() => setPalette(false)} />}
    </div>
  )
}
