import type { RepoParams } from '@/api/types'
import type {
  RequestQueuePageResponse,
  RequestQueueSection,
} from '@/api/types.generated'
import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { cn } from '@/lib/utils'
import { ChevronRight, CircleCheck, PanelLeftClose, PanelLeftOpen } from 'lucide-react'
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
  type QueueRow,
  type RequestWorkspaceListProps,
} from './request-workspace-list'
import {
  REQUEST_ATTENTION_GROUP_LABELS,
  REQUEST_ATTENTION_GROUP_ORDER,
  REQUEST_MERGE_STEP_LABELS,
  requestAgeLabel,
  requestAttentionLabel,
  type RequestAttentionGroup,
} from './request-workspace-model'
import { stackRequestRows } from './request-workspace-stack'
import { useRequestKeyboard } from './use-request-keyboard'
import './request-workspace-sidebar.css'

const RAIL_AVATARS = 6

const FOLDED_LABELS = {
  needs_you: 'need you',
  waiting: 'waiting',
  unclaimed: 'unclaimed',
  set_aside: 'set aside',
} as const

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
  const [hint, setHint] = useState<{ key: string; left: number; top: number; now: number } | null>(null)
  const [foldedStacks, setFoldedStacks] = useState<ReadonlySet<string>>(new Set())
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
    const target = (event.target as Element).closest<HTMLElement>(
      '[data-rail-hint], .request-workspace-row[data-rail]',
    )
    const anchor = target?.dataset.railHint ? target : target?.querySelector('.request-workspace-row-avatar')
    const key = target?.dataset.railHint ?? target?.dataset.requestId
    if (state !== 'closed' || !key || !anchor) return setHint(null)
    const { top, height } = anchor.getBoundingClientRect()
    setHint({
      key,
      left: event.currentTarget.getBoundingClientRect().right + 3,
      top: top + height / 2,
      now: Math.floor(Date.now() / 1000),
    })
  }

  const searching = query.trim().length > 0
  const common = {
    loading,
    skeleton,
    error,
    onRetry,
    onAction,
    params,
    pendingId,
    selectedId,
    foldedStacks,
    onFoldStack: (requestId: string, folded: boolean) =>
      setFoldedStacks((current) => {
        const next = new Set(current)
        if (folded) next.add(requestId)
        else next.delete(requestId)
        return next
      }),
  }
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
  const caughtUp = Boolean(pages) && !skeleton && needsYou.length === 0 && !activeHasMore
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
  )
  const foldedGroups = (Object.keys(FOLDED_LABELS) as (keyof typeof FOLDED_LABELS)[])
    .map((group) => ({
      group,
      count: grouped[group].filter((row) => row !== current && !railed.includes(row)).length,
    }))
    .filter(({ count }) => count > 0)
  const folded = foldedGroups.reduce((total, { count }) => total + count, 0)
  const hinted = hint && state === 'closed' ? railHint(hint.key, hint.now) : undefined
  function railHint(key: string, now: number) {
    if (key === 'needs-you') {
      const count = activeCount(needsYou.length)
      return {
        title: count === null ? REQUEST_ATTENTION_GROUP_LABELS.needs_you : `${REQUEST_ATTENTION_GROUP_LABELS.needs_you} · ${count}`,
        details: caughtUp ? [EMPTY_LABELS.needs_you] : [],
      }
    }
    if (key === 'folded')
      return {
        title: `${folded} more`,
        details: foldedGroups.map(({ group, count }) => `${count} ${FOLDED_LABELS[group]}`),
      }
    const row = allRows.find((row) => row.item.request.id === key)
    if (!row) return undefined
    const label = REQUEST_MERGE_STEP_LABELS[row.item.next_step] ?? requestAttentionLabel(row.item, true)
    return {
      title: row.item.request.title,
      details: [`${label} · ${requestAgeLabel(row.item.attention_at_unix, now, true)}`],
    }
  }
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
            <span className="block font-medium">{hinted.title}</span>
            {hinted.details.map((detail) => (
              <span className="block text-muted-foreground" key={detail}>
                {detail}
              </span>
            ))}
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
            {state === 'closed' ? <PanelLeftOpen aria-hidden="true" /> : <PanelLeftClose aria-hidden="true" />}
          </Button>
        </div>
        {viewingAs && <div className="request-workspace-viewing-as">{viewingAs}</div>}
        {actionError && (
          <p className="py-2 pr-4 pl-(--request-workspace-row-inset) text-xs text-danger-strong" role="alert">
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
                <GroupCount count={null} />
              </h2>
              <RequestWorkspaceListSkeleton rail />
            </section>
          ) : (
            <>
              {(maintainer || needsYou.length > 0) && (
                <section className="request-workspace-needs-you">
                  <h2 className="request-workspace-group-label text-foreground" data-rail-hint="needs-you">
                    <span>{REQUEST_ATTENTION_GROUP_LABELS.needs_you}</span>
                    {caughtUp && (
                      <CircleCheck aria-hidden="true" className="request-workspace-rail-caught-up" />
                    )}
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
                    data-rail-hint="folded"
                    onClick={openRail}
                    type="button"
                  >
                    {folded > 99 ? '99+' : `+${folded}`}
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
  return Object.fromEntries(
    REQUEST_ATTENTION_GROUP_ORDER.map((group) => [
      group,
      stackRequestRows(rows.filter((row) => row.item.attention.group === group)).map(({ row }) => row),
    ]),
  ) as Record<RequestAttentionGroup, QueueRow[]>
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
  return (
    <span className="tabular-nums">
      {count === null ? <TextSkeleton className="mx-auto" length="tiny" size="meta" /> : count}
    </span>
  )
}
