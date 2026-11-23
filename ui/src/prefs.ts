// Per-browser UI preferences. Storage can be unavailable (private mode), so every access is guarded.

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* ignore */
  }
}

export type Theme = 'dark' | 'light'

export function getTheme(): Theme {
  const saved = read('ft.theme')
  if (saved === 'dark' || saved === 'light') return saved
  return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

export function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute('data-bs-theme', theme)
  write('ft.theme', theme)
}

export function getLive(): boolean {
  return read('ft.live') !== '0'
}

export function setLive(on: boolean): void {
  write('ft.live', on ? '1' : '0')
}

export function getPref(key: string, fallback: string): string {
  return read(`ft.${key}`) ?? fallback
}

export function setPref(key: string, value: string): void {
  write(`ft.${key}`, value)
}
