import type { RouteParams } from './types.ts'

export const PAGE_IDS = ['status', 'assign', 'leases', 'pool', 'guests', 'settings'] as const
export type PageId = (typeof PAGE_IDS)[number]

export interface Route {
  page: PageId
  params: RouteParams
}

// Route = "#page?key=value&..." so pages can deep-link each other (e.g. Assign for a VM).
export function parseHash(): Route {
  const raw = window.location.hash.replace(/^#\/?/, '')
  const [page, query] = raw.split('?')
  const params = Object.fromEntries(new URLSearchParams(query || ''))
  const known = (PAGE_IDS as readonly string[]).includes(page)
  return { page: known ? (page as PageId) : 'status', params }
}

export function navigate(page: PageId, params: Record<string, string | number> = {}): void {
  const q = new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)])).toString()
  window.location.hash = `#${page}${q ? `?${q}` : ''}`
}
