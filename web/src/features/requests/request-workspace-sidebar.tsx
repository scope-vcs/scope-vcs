import type { RepoParams } from '@/api/types'
import type {
  RequestQueueItemResponse,
  RequestQueuePageResponse,
  RequestQueueSection,
} from '@/api/types.generated'
import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { ChevronLeft, ChevronRight } from 'lucide-react'
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
import {
  RequestWorkspaceList,
  RequestWorkspaceListSkeleton,
  type RequestWorkspaceListProps,
} from './request-workspace-list'
import {
  REQUEST_ATTENTION_GROUP_LABELS,
  REQUEST_ATTENTION_GROUP_ORDER,
  requestAgeLabel,
  requestAttentionLabel,
  type RequestAttentionGroup,
} from './request-workspace-model'
import { useRequestKeyboard } from './use-request-keyboard'
import './request-workspace-sidebar.css'

type QueueRow = { item: RequestQueueItemResponse; section: RequestQueueSection }

const RAIL_AVATARS = 6

const EMPTY_LABELS: Record<RequestAttentionGroup, string> = {
  needs_you: 'You’re caught up.',
  waiting: 'Nothing waiting on others.',
  unclaimed: 'Every request has a maintainer.',
  set_aside: 'Nothing set aside.',
  done: 'No closed or merged requests.',
}

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
  viewingAs,
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
  maintainer: boolean | null
  onLoadMore: (section: RequestQueueSection) => void
  params: RepoParams
  viewingAs?: ReactNode
}) {
  const aside = useRef<HTMLElement>(null)
  const [open, setOpen] = useState(false)
  const [closing, setClosing] = useState(false)
  const [hint, setHint] = useState<{ id: string; left: number; top: number; now: number } | null>(null)
  const state = collapsed ? (open ? 'open' : 'closed') : 'pinned'
  const openRail = useCallback(() => {
    setOpen(true)
    setClosing(false)
  }, [])
  const togglePinned = useCallback(() => {
    setOpen(false)
    setClosing(false)
    onCollapsedChange(!collapsed)
  }, [collapsed, onCollapsedChange])
  const toggleFocus = useCallback(() => {
    setOpen(false)
    setClosing(false)
    onFocusToggle()
  }, [onFocusToggle])
  const close = useCallback(() => {
    setClosing(true)
    if (query) onSearch('')
    if (document.activeElement instanceof HTMLElement && aside.current?.contains(document.activeElement))
      document.activeElement.blur()
  }, [onSearch, query])
  const toggle = state === 'open' ? close : togglePinned
  useEffect(() => {
    if (!closing) return
    let current = true
    const slides = aside.current?.getAnimations().filter(
      (animation) => animation instanceof CSSTransition && animation.transitionProperty === 'width',
    )
    Promise.all(slides?.map((slide) => slide.finished) ?? []).then(
      () => {
        if (!current) return
        setOpen(false)
        setClosing(false)
      },
      () => {},
    )
    return () => {
      current = false
    }
  }, [closing])
  useEffect(() => {
    if (state !== 'open' || closing) return
    function outside(event: PointerEvent) {
      if (!aside.current?.contains(event.target as Node)) close()
    }
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
  }, [close, closing, state])
  function click(event: MouseEvent) {
    const target = event.target as Element
    setHint(null)
    if ((state === 'closed' || closing) && !target.closest('a, button')) openRail()
    else if (state === 'open' && target.closest('a')) close()
  }
  function showHint(event: ReactPointerEvent | FocusEvent) {
    const row = (event.target as Element).closest<HTMLElement>('.request-workspace-row[data-rail]')
    const avatar = row?.querySelector('.request-workspace-row-avatar')
    if (state !== 'closed' || !row?.dataset.requestId || !avatar) return setHint(null)
    const { right, top, height } = avatar.getBoundingClientRect()
    setHint({
      id: row.dataset.requestId,
      left: right + 14,
      top: top + height / 2,
      now: Math.floor(Date.now() / 1000),
    })
  }

  const searching = query.trim().length > 0
  const common = { loading, skeleton, error, onRetry, onAction, params, pendingId, selectedId }
  const rows = (section: RequestQueueSection): QueueRow[] =>
    pages?.[section].requests.map((item) => ({ item, section })) ?? []
  const allRows = REQUEST_QUEUE_SECTION_ORDER.flatMap(rows)
  const grouped = groupRows(allRows)
  useRequestKeyboard({
    focus,
    onAction,
    onCollapseToggle: toggle,
    onFocusToggle: toggleFocus,
    rows: new Map(allRows.map((row) => [row.item.request.id, row])),
    selectedId,
  })
  const nextSection = REQUEST_QUEUE_SECTION_ORDER.find((section) => pages?.[section].next_cursor)
  const activeHasMore = Boolean(pages?.active.next_cursor)
  const activeCount = (value: number) => pages ? `${value}${activeHasMore ? '+' : ''}` : null
  const needsYou = grouped.needs_you
  const current = searching
    ? undefined
    : allRows.find((row) => row.item.request.id === selectedId && !needsYou.includes(row))
  const moved = (items: QueueRow[]) => Number(current !== undefined && items.includes(current))
  const waiting = grouped.waiting.filter((row) => row !== current)
  const waitingCount = activeCount(grouped.waiting.length - moved(grouped.waiting))
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
        count: pages ? count(pages[group], moved(grouped[group])) : null,
        emptyLabel: EMPTY_LABELS[group],
      }),
    ),
  ]
  const railed = needsYou.filter(
    (row, index) => index < RAIL_AVATARS || row.item.request.id === selectedId,
  ).length
  const folded = allRows.length - railed - Number(Boolean(current))
  const hinted = hint && state === 'closed' ? allRows.find((row) => row.item.request.id === hint.id) : undefined
  return (
    <aside
      aria-label="Requests workspace"
      className="request-workspace-sidebar"
      data-closing={closing || undefined}
      data-state={state}
      onBlur={() => setHint(null)}
      onClick={click}
      onFocus={showHint}
      onPointerLeave={() => setHint(null)}
      onPointerOver={showHint}
      ref={aside}
    >
      {hint &&
        hinted &&
        createPortal(
          <div
            aria-hidden="true"
            className="request-workspace-rail-hint"
            style={{ left: hint.left, top: hint.top }}
          >
            <span className="block font-medium">{hinted.item.request.title}</span>
            <span className="block text-muted-foreground">
              {requestAttentionLabel(hinted.item, true)} ·{' '}
              {requestAgeLabel(hinted.item.attention_at_unix, hint.now, true)}
            </span>
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
            aria-label={state === 'closed' ? 'Expand requests sidebar' : 'Collapse requests sidebar'}
            className="request-workspace-sidebar-toggle text-muted-foreground"
            onClick={toggle}
            size="icon-sm"
            title={state === 'closed' ? 'Expand requests sidebar' : 'Collapse requests sidebar'}
            type="button"
            variant="ghost"
          >
            {state === 'closed' ? <ChevronRight aria-hidden="true" /> : <ChevronLeft aria-hidden="true" />}
          </Button>
        </div>
        {viewingAs && <div className="request-workspace-viewing-as">{viewingAs}</div>}
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
          ) : maintainer === null ? (
            <section className="request-workspace-needs-you">
              <h2 aria-hidden="true" className="request-workspace-group-label">
                <span>
                  <TextSkeleton length="short" size="meta" />
                </span>
                <span>
                  <TextSkeleton className="mx-auto" length="tiny" size="meta" />
                </span>
              </h2>
              <RequestWorkspaceListSkeleton rail />
            </section>
          ) : (
            <>
              {(maintainer || needsYou.length > 0) && (
                <section className="request-workspace-needs-you">
                  <h2 className="request-workspace-group-label text-foreground">
                    <span>{REQUEST_ATTENTION_GROUP_LABELS.needs_you}</span>
                    <GroupCount count={activeCount(needsYou.length)} />
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
                    <GroupCount count={waitingCount} />
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

function groupRows(rows: QueueRow[]) {
  const grouped = Object.fromEntries(
    REQUEST_ATTENTION_GROUP_ORDER.map((group) => [group, [] as QueueRow[]]),
  ) as Record<RequestAttentionGroup, QueueRow[]>
  for (const row of rows) {
    grouped[row.item.attention.group].push(row)
  }
  return grouped
}

function RequestWorkspaceDisclosure({
  children,
  count,
  label,
}: {
  children: ReactNode
  count: string | null
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
        <span className="ml-auto flex items-center">
          <GroupCount count={count} />
        </span>
      </button>
      <div hidden={!open} id={id}>
        {children}
      </div>
    </section>
  )
}

function count(page: RequestQueuePageResponse, moved: number) {
  return `${page.requests.length - moved}${page.next_cursor ? '+' : ''}`
}

function GroupCount({ count }: { count: string | null }) {
  return count === null
    ? <TextSkeleton length="tiny" size="meta" />
    : <span className="tabular-nums">{count}</span>
}
