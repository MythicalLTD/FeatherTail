// Tiny fetch wrapper. Auth is a HttpOnly session cookie set by /api/v1/auth/login.

let onUnauthorized: () => void = () => {}
export function setUnauthorizedHandler(fn: () => void): void {
  onUnauthorized = fn
}

export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api/v1${path}`, {
    method,
    credentials: 'same-origin',
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : {},
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  let data: unknown = null
  const text = await res.text()
  if (text) {
    try {
      data = JSON.parse(text)
    } catch {
      data = { error: text }
    }
  }
  if (!res.ok) {
    if (res.status === 401 && !path.startsWith('/auth/')) onUnauthorized()
    const message = (data as { error?: string } | null)?.error
    throw new ApiError(res.status, message || `${res.status} ${res.statusText}`)
  }
  return data as T
}

export const api = {
  get: <T>(p: string) => request<T>('GET', p),
  post: <T = unknown>(p: string, b: unknown = {}) => request<T>('POST', p, b),
  put: <T = unknown>(p: string, b: unknown = {}) => request<T>('PUT', p, b),
  del: <T = unknown>(p: string) => request<T>('DELETE', p),
}

export const enc = encodeURIComponent

export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message
  return String(e)
}

export function fmtTime(unix: number | null | undefined): string {
  if (!unix) return '—'
  return new Date(unix * 1000).toLocaleString()
}

export function fmtRelative(unix: number | null | undefined): string {
  if (!unix) return '—'
  const diff = unix - Math.floor(Date.now() / 1000)
  const abs = Math.abs(diff)
  const units: [number, string][] = [
    [86400, 'd'],
    [3600, 'h'],
    [60, 'm'],
  ]
  let label = `${abs}s`
  for (const [secs, u] of units) {
    if (abs >= secs) {
      label = `${Math.floor(abs / secs)}${u}`
      break
    }
  }
  return diff >= 0 ? `in ${label}` : `${label} ago`
}

export function ipToInt(ip: string): number {
  return ip.split('.').reduce((acc, o) => acc * 256 + Number(o), 0)
}

export function splitList(value: string): string[] {
  return value.split(/[\s,]+/).filter(Boolean)
}

// ---------------------------------------------------------------- refresh signal
const bus = new EventTarget()

/** Every useLoader re-fetches (silently) when this fires. */
export function refreshAll(): void {
  bus.dispatchEvent(new Event('refresh'))
}

export function onRefresh(fn: () => void): () => void {
  const handler = () => fn()
  bus.addEventListener('refresh', handler)
  return () => bus.removeEventListener('refresh', handler)
}

// ---------------------------------------------------------------- csv
export function downloadCsv(filename: string, rows: (string | number | null | undefined)[][]): void {
  const esc = (v: string | number | null | undefined) => {
    const s = v == null ? '' : String(v)
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
  }
  const blob = new Blob([rows.map((r) => r.map(esc).join(',')).join('\n') + '\n'], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
