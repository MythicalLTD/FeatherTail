import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { api } from './api.ts'
import { useEscape } from './components.tsx'
import { navigate, type PageId } from './router.ts'
import type { GuestView, LeaseView, PoolEntry } from './types.ts'

interface Item {
  group: string
  icon: string
  label: string
  sub?: string
  haystack: string
  run: () => void
}

interface Props {
  pages: { id: PageId; label: string; icon: string }[]
  onClose: () => void
}

export default function CommandPalette({ pages, onClose }: Props) {
  const [q, setQ] = useState('')
  const [cursor, setCursor] = useState(0)
  const [guests, setGuests] = useState<GuestView[]>([])
  const [leases, setLeases] = useState<LeaseView[]>([])
  const [pool, setPool] = useState<PoolEntry[]>([])
  const listRef = useRef<HTMLDivElement>(null)
  useEscape(onClose)

  useEffect(() => {
    api.get<GuestView[]>('/admin/guests').then(setGuests, () => {})
    api.get<LeaseView[]>('/admin/leases').then(setLeases, () => {})
    api.get<PoolEntry[]>('/admin/pool').then(setPool, () => {})
  }, [])

  const items = useMemo<Item[]>(() => {
    const go = (page: PageId, params: Record<string, string | number> = {}) => () => {
      navigate(page, params)
      onClose()
    }
    const out: Item[] = pages.map((p) => ({
      group: 'Go to',
      icon: p.icon,
      label: p.label,
      haystack: p.label.toLowerCase(),
      run: go(p.id),
    }))
    for (const g of guests.filter((g) => !g.template)) {
      const ips = g.nics.map((n) => n.lease?.ip).filter(Boolean).join(' ')
      out.push({
        group: 'Guests',
        icon: g.kind === 'lxc' ? 'box' : 'pc-display',
        label: `${g.vmid} · ${g.name}`,
        sub: ips || g.nics.map((n) => n.bridge).join(', '),
        haystack: `${g.vmid} ${g.name} ${g.node} ${ips} ${g.nics.map((n) => `${n.mac} ${n.bridge}`).join(' ')}`.toLowerCase(),
        run: go('assign', { vmid: g.vmid }),
      })
    }
    for (const l of leases) {
      out.push({
        group: 'Leases',
        icon: 'hdd-network',
        label: l.ip,
        sub: [l.vmid != null ? `${l.vmid} ${l.guest_name ?? ''}` : l.hostname, l.mac].filter(Boolean).join(' · '),
        haystack: `${l.ip} ${l.mac} ${l.vmid ?? ''} ${l.guest_name ?? ''} ${l.hostname ?? ''}`.toLowerCase(),
        run: go('leases', { q: l.ip }),
      })
    }
    for (const p of pool.filter((p) => !p.lease)) {
      out.push({
        group: 'Free IPs',
        icon: 'circle',
        label: p.ip,
        sub: p.bridge,
        haystack: `${p.ip} ${p.bridge} free`.toLowerCase(),
        run: go('pool', { bridge: p.bridge, q: p.ip }),
      })
    }
    return out
  }, [pages, guests, leases, pool, onClose])

  const results = useMemo(() => {
    const terms = q.trim().toLowerCase().split(/\s+/).filter(Boolean)
    if (!terms.length) return items.filter((i) => i.group === 'Go to' || i.group === 'Guests').slice(0, 14)
    const hits = items.filter((i) => terms.every((t) => i.haystack.includes(t)))
    // Keep groups together, cap each group.
    const byGroup = new Map<string, Item[]>()
    for (const h of hits) {
      const arr = byGroup.get(h.group) ?? []
      if (arr.length < 8) arr.push(h)
      byGroup.set(h.group, arr)
    }
    return [...byGroup.values()].flat()
  }, [items, q])

  useEffect(() => setCursor(0), [q])
  useEffect(() => {
    listRef.current?.querySelector('.list-group-item.active')?.scrollIntoView({ block: 'nearest' })
  }, [cursor])

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setCursor((c) => Math.min(results.length - 1, c + 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setCursor((c) => Math.max(0, c - 1))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      results[cursor]?.run()
    }
  }

  let lastGroup = ''
  return (
    <>
      <div className="modal-backdrop show" />
      <div className="modal d-block" tabIndex={-1} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
        <div className="modal-dialog modal-lg">
          <div className="modal-content">
            <div className="modal-header">
              <div className="input-group">
                <span className="input-group-text">
                  <i className="bi bi-search" />
                </span>
                <input
                  className="form-control"
                  autoFocus
                  value={q}
                  onChange={(e) => setQ(e.target.value)}
                  onKeyDown={onKey}
                  placeholder="VMID, name, IP, MAC, bridge…"
                />
              </div>
            </div>
            <div className="modal-body p-0 overflow-auto" style={{ maxHeight: 440 }} ref={listRef}>
              {results.length === 0 && <div className="text-center text-body-secondary py-4">No matches</div>}
              <div className="list-group list-group-flush">
                {results.map((r, i) => {
                  const header =
                    r.group !== lastGroup ? (
                      <div className="list-group-item bg-body-tertiary small text-body-secondary text-uppercase fw-semibold py-1">{r.group}</div>
                    ) : null
                  lastGroup = r.group
                  return (
                    <div key={`${r.group}-${r.label}-${i}`}>
                      {header}
                      <button
                        type="button"
                        className={`list-group-item list-group-item-action d-flex align-items-center gap-2 ${i === cursor ? 'active' : ''}`}
                        onMouseEnter={() => setCursor(i)}
                        onClick={r.run}
                      >
                        <i className={`bi bi-${r.icon}`} />
                        <span className={r.group === 'Go to' ? '' : 'font-monospace'}>{r.label}</span>
                        {r.sub && <small className={`ms-auto text-truncate ${i === cursor ? '' : 'text-body-secondary'}`}>{r.sub}</small>}
                      </button>
                    </div>
                  )
                })}
              </div>
            </div>
          </div>
        </div>
      </div>
    </>
  )
}
