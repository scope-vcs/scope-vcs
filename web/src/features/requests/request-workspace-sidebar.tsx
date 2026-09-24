import type { RepoParams } from '@/api/types'
import type {
  RequestQueueItemResponse,
  RequestQueuePageResponse,
  RequestQueueSection,
} from '@/api/types.generated'
import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { ChevronRight, Pin } from 'lucide-react'
import { createPortal } from 'react-dom'
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type FocusEvent,
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from 'react'
import { REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages } from './request-list-model'
import { RequestWorkspaceList, type RequestWorkspaceListProps } from './request-workspace-list'
import {
  REQUEST_ATTENTION_GROUP_LABELS,
  REQUEST_ATTENTION_GROUP_ORDER,
  requestAttentionGroup,
  requestAttentionLabel,
  type RequestAttentionGroup,
} from './request-workspace-model'
import { useRequestKeyboard } from './use-request-keyboard'
import './request-workspace-sidebar.css'

type QueueRow = { item: RequestQueueItemResponse; section: RequestQueueSection }

/** Needs-you avatars the closed rail shows before the rest fold into its count. */
const RAIL_AVATARS = 6

const EMPTY_LABELS: Record<RequestAttentionGroup, string> = {
  needs_you: 'You’re caught up.',
  waiting: 'Nothing waiting on others.',
  unclaimed: 'Every request has a maintainer.',
  set_aside: 'Nothing set aside.',
  done: 'No closed or merged requests.',
}

/**
 * The requests sidebar in one of three states. Pinned is the resizable
 * sidebar. Closed is a 54px rail showing the avatars of what needs the viewer.
 * Open is that rail widened over the page until the viewer picks a request,
 * clicks away or presses Escape. All three draw the same list, so the rail's
 * avatars stay put as it opens and nothing appears twice.
 */
export function RequestWorkspaceSidebar({
  pages,
  collapsed,
  onCollapsedChange,
  query,
  onSearch,
  loading,
  skeleton,
  error,
  actionError,
  maintainer,
  onRetry,
  onLoadMore,
  onAction,
  params,
  pendingId,
  selectedId,
  focus,
  onFocusToggle,
}: Pick<
  RequestWorkspaceListProps,
  'loading' | 'skeleton' | 'error' | 'onRetry' | 'onAction' | 'pendingId' | 'selectedId'
> & {
  focus: boolean
  onFocusToggle: () => void
  pages: RequestQueuePages | undefined
  collapsed: boolean
  onCollapsedChange: (collapsed: boolean) => void
  query: string
  onSearch: (query: string) => void
  actionError: string | null
  maintainer: boolean
  onLoadMore: (section: RequestQueueSection) => void
  params: RepoParams
}) {
  const aside = useRef<HTMLElement>(null)
  const [open, setOpen] = useState(false)
  const [hint, setHint] = useState<{ id: string; left: number; top: number } | null>(null)
  const state = collapsed ? (open ? 'open' : 'closed') : 'pinned'
  const openRail = useCallback(() => setOpen(true), [])
  const togglePinned = useCallback(() => {
    setOpen(false)
    onCollapsedChange(!collapsed)
  }, [collapsed, onCollapsedChange])
  // Focus mode keeps the narrow rail, not the rail opened over the page.
  const toggleFocus = useCallback(() => {
    setOpen(false)
    onFocusToggle()
  }, [onFocusToggle])
  const close = useCallback(() => {
    setOpen(false)
    if (query) onSearch('')
    // Focus left inside would sit in a search box or row the rail now hides.
    if (document.activeElement instanceof HTMLElement && aside.current?.contains(document.activeElement))
      document.activeElement.blur()
  }, [onSearch, query])
  useEffect(() => {
    if (state !== 'open') return
    function outside(event: PointerEvent) {
      if (!aside.current?.contains(event.target as Node)) close()
    }
    // Escape belongs to the open rail unless a menu or dialog is up: closing
    // clears the search too, and the key must not reach focus mode or the
    // search box, which refocuses itself.
    function escape(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      if (document.querySelector(':popover-open, [role="dialog"], [role="alertdialog"]')) return
      event.preventDefault()
      event.stopPropagation()
      close()
    }
    document.addEventListener('pointerdown', outside)
    document.addEventListener('keydown', escape, true)
    return () => {
      document.removeEventListener('pointerdown', outside)
      document.removeEventListener('keydown', escape, true)
    }
  }, [close, state])
  // Avatars on the closed rail navigate; anywhere else on it opens the rail.
  // Picking a request from the open rail closes it.
  function click(event: MouseEvent) {
    const link = (event.target as Element).closest('a')
    setHint(null)
    if (state === 'closed' && !link) setOpen(true)
    else if (state === 'open' && link) close()
  }
  // The closed rail shows only avatars, so hovering or focusing one names its
  // request beside the rail.
  function showHint(event: ReactPointerEvent | FocusEvent) {
    const row = (event.target as Element).closest<HTMLElement>('.request-workspace-row[data-rail]')
    const avatar = row?.querySelector('.request-workspace-row-avatar')
    if (state !== 'closed' || !row?.dataset.requestId || !avatar) return setHint(null)
    const { right, top, height } = avatar.getBoundingClientRect()
    setHint({ id: row.dataset.requestId, left: right + 14, top: top + height / 2 })
  }

  const searching = query.trim().length > 0
  const common = { loading, skeleton, error, maintainer, onRetry, onAction, params, pendingId, selectedId }
  const rows = (section: RequestQueueSection): QueueRow[] =>
    pages?.[section].requests.map((item) => ({ item, section })) ?? []
  const allRows = REQUEST_QUEUE_SECTION_ORDER.flatMap(rows)
  const grouped = groupRows(allRows, maintainer)
  useRequestKeyboard({
    focus,
    onAction,
    onCollapseToggle: togglePinned,
    onFocusToggle: toggleFocus,
    rows: new Map(allRows.map((row) => [row.item.request.id, row])),
    selectedId,
  })
  const nextSection = REQUEST_QUEUE_SECTION_ORDER.find((section) => pages?.[section].next_cursor)
  const activeHasMore = Boolean(pages?.active.next_cursor)
  // Active rows page, so loaded lengths are floors until the last page is in.
  const activeCount = (value: number) => `${value}${activeHasMore ? '+' : ''}`
  const needsYou = grouped.needs_you
  // The open request keeps a slot on the rail when it is not one of the
  // viewer's, and leaves its group so it is never listed twice.
  const current = searching
    ? undefined
    : allRows.find((row) => row.item.request.id === selectedId && !needsYou.includes(row))
  const moved = (items: QueueRow[]) => Number(current !== undefined && items.includes(current))
  const waiting = grouped.waiting.filter((row) => row !== current)
  const waitingCount = activeCount(grouped.waiting.length - moved(grouped.waiting))
  // A reader's open requests are their whole queue, so they stay listed.
  // Unclaimed and Set aside only ever hold maintainer placements, so readers
  // see just their work and the finished history.
  const disclosures = [
    ...(maintainer
      ? [
          {
            group: 'waiting',
            section: 'active',
            label: REQUEST_ATTENTION_GROUP_LABELS.waiting,
            count: waitingCount,
            emptyLabel: EMPTY_LABELS.waiting,
          } as const,
        ]
      : []),
    ...(maintainer ? (['unclaimed', 'set_aside', 'done'] as const) : (['done'] as const)).map(
      (group) => ({
        group,
        section: group,
        label: REQUEST_ATTENTION_GROUP_LABELS[group],
        count: count(pages?.[group], moved(grouped[group])),
        emptyLabel: EMPTY_LABELS[group],
      }),
    ),
  ]
  // The cap never hides the open request's avatar.
  const railed = needsYou.filter(
    (row, index) => index < RAIL_AVATARS || row.item.request.id === selectedId,
  ).length
  const folded = allRows.length - railed - Number(Boolean(current))
  const hinted = hint && state === 'closed' ? allRows.find((row) => row.item.request.id === hint.id) : undefined
  return (
    <aside
      aria-label="Requests workspace"
      className="request-workspace-sidebar"
      data-state={state}
      onBlur={() => setHint(null)}
      onClick={click}
      onFocus={(event) => {
        if (state === 'closed' && event.target instanceof HTMLInputElement) setOpen(true)
        else showHint(event)
      }}
      onPointerLeave={() => setHint(null)}
      onPointerOver={showHint}
      ref={aside}
    >
      {/* On the body: the page's size container would otherwise place and clip it. */}
      {hint &&
        hinted &&
        createPortal(
          <div
            aria-hidden="true"
            className="request-workspace-rail-hint"
            style={{ left: hint.left, top: hint.top }}
          >
            <span className="block font-medium">{hinted.item.request.title}</span>
            <span className="block text-muted-foreground">{requestAttentionLabel(hinted.item, true)}</span>
          </div>,
          document.body,
        )}
      <div className="request-workspace-sidebar-inner">
        <div className="request-workspace-sidebar-tools">
          <NavigationSearch
            clearLabel="Clear request search"
            label="Search requests"
            onChange={onSearch}
            onOpen={openRail}
            placeholder="Search requests"
            status={loading && searching ? 'Searching requests' : undefined}
            value={query}
          />
          <Button
            aria-label={state === 'pinned' ? 'Unpin requests sidebar' : 'Pin requests sidebar'}
            aria-pressed={state === 'pinned'}
            className="request-workspace-pin text-muted-foreground aria-pressed:text-foreground"
            onClick={togglePinned}
            size="icon-sm"
            title={state === 'pinned' ? 'Unpin requests sidebar' : 'Keep the requests sidebar open'}
            type="button"
            variant="ghost"
          >
            <Pin aria-hidden="true" className={cn(state === 'pinned' && 'fill-current')} />
          </Button>
        </div>
        {actionError && (
          <p className="px-4 py-2 text-[11px] text-danger-strong" role="alert">
            {actionError}
          </p>
        )}
        <div aria-busy={loading} className="request-workspace-scroll">
          {searching ? (
            <RequestWorkspaceList
              {...common}
              emptyLabel="No matching requests."
              hasMore={Boolean(nextSection)}
              items={allRows}
              onLoadMore={() => {
                if (nextSection) onLoadMore(nextSection)
              }}
            />
          ) : (
            <>
              {(maintainer || needsYou.length > 0) && (
                <section className="request-workspace-needs-you">
                  <h2 className="request-workspace-group-label text-foreground">
                    <span>{REQUEST_ATTENTION_GROUP_LABELS.needs_you}</span>
                    <span className="tabular-nums">{activeCount(needsYou.length)}</span>
                  </h2>
                  <RequestWorkspaceList
                    {...common}
                    emptyLabel={EMPTY_LABELS.needs_you}
                    hasMore={activeHasMore && grouped.waiting.length === 0}
                    items={needsYou}
                    onLoadMore={() => onLoadMore('active')}
                    rail
                  />
                </section>
              )}
              {current && (
                <section aria-label="Open request" className="request-workspace-current">
                  <hr className="request-workspace-rail-rule" />
                  <RequestWorkspaceList
                    {...common}
                    emptyLabel=""
                    hasMore={false}
                    items={[current]}
                    onLoadMore={() => {}}
                    rail
                  />
                </section>
              )}
              {state === 'closed' && folded > 0 && (
                <>
                  <hr className="request-workspace-rail-rule" />
                  <button
                    aria-label={`Show ${folded} more requests`}
                    className="request-workspace-rail-more"
                    onClick={openRail}
                    type="button"
                  >
                    +{Math.min(folded, 99)}
                  </button>
                </>
              )}
              {!maintainer && (waiting.length > 0 || needsYou.length === 0) && (
                <section className="request-workspace-open">
                  <h2 className="request-workspace-group-label text-muted-foreground">
                    <span>Open</span>
                    <span className="tabular-nums">{waitingCount}</span>
                  </h2>
                  <RequestWorkspaceList
                    {...common}
                    emptyLabel={EMPTY_LABELS.needs_you}
                    hasMore={activeHasMore}
                    items={waiting}
                    onLoadMore={() => onLoadMore('active')}
                  />
                </section>
              )}
              <div className="request-workspace-disclosures">
                {disclosures.map(({ group, section, label, count, emptyLabel }) => (
                  <RequestWorkspaceDisclosure count={count} key={group} label={label}>
                    <RequestWorkspaceList
                      {...common}
                      emptyLabel={emptyLabel}
                      hasMore={Boolean(pages?.[section].next_cursor)}
                      items={grouped[group].filter((row) => row !== current)}
                      onLoadMore={() => onLoadMore(section)}
                    />
                  </RequestWorkspaceDisclosure>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </aside>
  )
}

function groupRows(rows: QueueRow[], maintainer: boolean) {
  const grouped = Object.fromEntries(
    REQUEST_ATTENTION_GROUP_ORDER.map((group) => [group, [] as QueueRow[]]),
  ) as Record<RequestAttentionGroup, QueueRow[]>
  for (const row of rows) {
    grouped[requestAttentionGroup(row.section, row.item.attention.reason, maintainer)].push(row)
  }
  return grouped
}

function RequestWorkspaceDisclosure({
  children,
  count,
  label,
}: {
  children: ReactNode
  count: string
  label: string
}) {
  const [open, setOpen] = useState(false)
  const id = useId()
  return (
    <section>
      <button
        aria-controls={id}
        aria-expanded={open}
        className="request-workspace-group-label request-workspace-group-toggle"
        onClick={() => setOpen(!open)}
        type="button"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn(
            'size-3 transition-transform motion-reduce:transition-none',
            open && 'rotate-90',
          )}
        />
        <span>{label}</span>
        <span className="ml-auto tabular-nums">{count}</span>
      </button>
      <div hidden={!open} id={id}>
        {children}
      </div>
    </section>
  )
}

function count(page: RequestQueuePageResponse | undefined, moved: number) {
  return `${(page?.requests.length ?? 0) - moved}${page?.next_cursor ? '+' : ''}`
}
