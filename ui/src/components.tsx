import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type DependencyList,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from 'react'
import { errorMessage, onRefresh } from './api.ts'

// ---------------------------------------------------------------- toasts
export type Variant = 'success' | 'danger' | 'warning' | 'info'
type ToastFn = (message: ReactNode, variant?: Variant) => void

const TOAST_ICONS: Record<Variant, string> = {
  success: 'check-circle-fill text-success',
  danger: 'x-octagon-fill text-danger',
  warning: 'exclamation-triangle-fill text-warning',
  info: 'info-circle-fill text-primary',
}

const ToastCtx = createContext<ToastFn>(() => {})
export const useToast = () => useContext(ToastCtx)

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<{ id: string; message: ReactNode; variant: Variant }[]>([])
  const close = (id: string) => setToasts((t) => t.filter((x) => x.id !== id))
  const push = useCallback<ToastFn>((message, variant = 'success') => {
    const id = Math.random().toString(36).slice(2)
    setToasts((t) => [...t, { id, message, variant }])
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), variant === 'danger' ? 7000 : 3500)
  }, [])
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toast-container position-fixed bottom-0 end-0 p-3">
        {toasts.map((t) => (
          <div key={t.id} className="toast show" role="status">
            <div className="toast-body d-flex align-items-center gap-2">
              <i className={`bi bi-${TOAST_ICONS[t.variant]}`} />
              <div className="me-auto">{t.message}</div>
              <button type="button" className="btn-close" onClick={() => close(t.id)} aria-label="Close" />
            </div>
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  )
}

// ---------------------------------------------------------------- modal
interface ModalProps {
  title: ReactNode
  onClose: () => void
  children: ReactNode
  footer?: ReactNode
  size?: '' | 'modal-sm' | 'modal-lg' | 'modal-xl'
}

export function Modal({ title, onClose, children, footer, size = '' }: ModalProps) {
  useEscape(onClose)
  return (
    <>
      <div className="modal-backdrop show" />
      <div className="modal d-block" tabIndex={-1} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
        <div className={`modal-dialog ${size}`}>
          <div className="modal-content">
            <div className="modal-header">
              <h5 className="modal-title">{title}</h5>
              <button type="button" className="btn-close" onClick={onClose} aria-label="Close" />
            </div>
            <div className="modal-body">{children}</div>
            {footer && <div className="modal-footer">{footer}</div>}
          </div>
        </div>
      </div>
    </>
  )
}

export function useEscape(fn: () => void) {
  const ref = useRef(fn)
  ref.current = fn
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') ref.current()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

// ---------------------------------------------------------------- confirm
interface ConfirmOptions {
  title?: string
  confirmLabel?: string
  variant?: 'danger' | 'primary' | 'warning'
}
type ConfirmFn = (message: ReactNode, options?: ConfirmOptions) => Promise<boolean>

interface ConfirmState extends Required<ConfirmOptions> {
  message: ReactNode
  resolve: (ok: boolean) => void
}

const ConfirmCtx = createContext<ConfirmFn>(async () => false)
export const useConfirm = () => useContext(ConfirmCtx)

export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<ConfirmState | null>(null)
  const confirm = useCallback<ConfirmFn>(
    (message, { title = 'Confirm', confirmLabel = 'Confirm', variant = 'danger' } = {}) =>
      new Promise<boolean>((resolve) => setState({ message, title, confirmLabel, variant, resolve })),
    [],
  )
  const close = (v: boolean) => {
    state?.resolve(v)
    setState(null)
  }
  return (
    <ConfirmCtx.Provider value={confirm}>
      {children}
      {state && (
        <Modal
          title={state.title}
          onClose={() => close(false)}
          footer={
            <>
              <button className="btn btn-secondary" onClick={() => close(false)}>
                Cancel
              </button>
              <button className={`btn btn-${state.variant}`} onClick={() => close(true)} autoFocus>
                {state.confirmLabel}
              </button>
            </>
          }
        >
          {state.message}
        </Modal>
      )}
    </ConfirmCtx.Provider>
  )
}

// ---------------------------------------------------------------- small pieces
const STATUS: Record<string, string> = {
  running: 'success',
  active: 'success',
  stopped: 'secondary',
  paused: 'warning',
  released: 'warning',
  expired: 'danger',
}

export function StatusPill({ status }: { status: string | null | undefined }) {
  const s = status || 'unknown'
  return <span className={`badge text-bg-${STATUS[s] || 'secondary'}`}>{s}</span>
}

/** Small status dot (Bootstrap icon). */
export function Dot({ tone }: { tone: 'success' | 'secondary' | 'warning' | 'danger' | 'primary' }) {
  return <i className={`bi bi-circle-fill text-${tone} me-1`} style={{ fontSize: '0.55rem', verticalAlign: 'middle' }} />
}

export function Spinner() {
  return (
    <div className="text-center text-body-secondary py-5">
      <span className="spinner-border spinner-border-sm" />
    </div>
  )
}

/** Bootstrap placeholder rows while loading. */
export function Skeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="placeholder-wave p-3">
      {Array.from({ length: rows }, (_, i) => (
        <span key={i} className="placeholder col-12 placeholder-lg mb-2 d-block" />
      ))}
    </div>
  )
}

export function ErrorBox({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  if (!error) return null
  return (
    <div className="alert alert-danger d-flex justify-content-between align-items-center gap-2">
      <span>
        <i className="bi bi-x-octagon me-2" />
        {errorMessage(error)}
      </span>
      {onRetry && (
        <button className="btn btn-sm btn-outline-danger" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
  )
}

export function PageHead({ title, meta, children }: { title: ReactNode; meta?: ReactNode; children?: ReactNode }) {
  return (
    <div className="d-flex flex-wrap justify-content-between align-items-center gap-2 mb-4">
      <div>
        <h3 className="mb-1">{title}</h3>
        {meta && <div className="d-flex flex-wrap gap-1 align-items-center">{meta}</div>}
      </div>
      {children && <div className="d-flex flex-wrap gap-2">{children}</div>}
    </div>
  )
}

export function Empty({ icon = 'inbox', children }: { icon?: string; children: ReactNode }) {
  return (
    <div className="text-center text-body-secondary py-5">
      <i className={`bi bi-${icon} fs-2 d-block mb-2`} />
      {children}
    </div>
  )
}

export function Meter({ value, max, height = 6 }: { value: number; max: number; height?: number }) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0
  const tone = pct >= 95 ? 'bg-danger' : pct >= 80 ? 'bg-warning' : ''
  return (
    <div className="progress" role="progressbar" aria-valuenow={value} aria-valuemin={0} aria-valuemax={max} style={{ height }} title={`${value} / ${max}`}>
      <div className={`progress-bar ${tone}`} style={{ width: `${pct}%` }} />
    </div>
  )
}

export function Tile(props: { label: string; icon: string; value: ReactNode; hint?: ReactNode; onClick?: () => void }) {
  return (
    <div className={`card h-100 ${props.onClick ? 'cursor-pointer' : ''}`} onClick={props.onClick}>
      <div className="card-body">
        <div className="d-flex justify-content-between text-body-secondary small mb-1">
          <span>{props.label}</span>
          <i className={`bi bi-${props.icon}`} />
        </div>
        <div className="fs-3 fw-semibold">{props.value}</div>
        {props.hint && <div className="small text-body-secondary">{props.hint}</div>}
      </div>
    </div>
  )
}

export interface SegOption<T extends string> {
  value: T
  label: ReactNode
  icon?: string
  count?: number
}

export function Seg<T extends string>({ value, options, onChange }: { value: T; options: SegOption<T>[]; onChange: (v: T) => void }) {
  return (
    <div className="btn-group btn-group-sm" role="group">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          className={`btn btn-outline-secondary ${o.value === value ? 'active' : ''}`}
          onClick={() => onChange(o.value)}
        >
          {o.icon && <i className={`bi bi-${o.icon}${o.label ? ' me-1' : ''}`} />}
          {o.label}
          {o.count !== undefined && <span className="badge text-bg-secondary ms-1">{o.count}</span>}
        </button>
      ))}
    </div>
  )
}

export function SearchInput({ value, onChange, placeholder = 'Search' }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <div className="input-group input-group-sm" style={{ maxWidth: 360 }}>
      <span className="input-group-text">
        <i className="bi bi-search" />
      </span>
      <input className="form-control" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
    </div>
  )
}

/** Click to copy. */
export function Copy({ text, children, className = '' }: { text: string; children?: ReactNode; className?: string }) {
  const toast = useToast()
  return (
    <span
      className={`copyable ${className}`}
      title="Copy"
      onClick={(e) => {
        e.stopPropagation()
        navigator.clipboard?.writeText(text).then(
          () => toast(<>Copied <code>{text}</code></>, 'info'),
          () => {},
        )
      }}
    >
      {children ?? text}
    </span>
  )
}

// ---------------------------------------------------------------- data loading
export interface Loader<T> {
  data: T | null
  error: unknown
  loading: boolean
  reload: () => Promise<void>
  setData: Dispatch<SetStateAction<T | null>>
}

/**
 * Loads data with a reload function; keeps previous data while reloading and reloads
 * silently on the global refresh signal (auto-refresh and after any change).
 */
export function useLoader<T>(fn: () => Promise<T>, deps: DependencyList = []): Loader<T> {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<unknown>(null)
  const [loading, setLoading] = useState(true)
  const load = useCallback(async () => {
    setLoading(true)
    try {
      setData(await fn())
      setError(null)
    } catch (e) {
      setError(e)
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)
  useEffect(() => {
    load()
    return onRefresh(load)
  }, [load])
  return { data, error, loading, reload: load, setData }
}
