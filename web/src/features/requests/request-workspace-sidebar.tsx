import type { RepoParams } from '@/api/types'
import type {
  RequestQueueItemResponse,
  RequestQueuePageResponse,
  RequestQueueSection,
} from '@/api/types.generated'
import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useCallback, useId, useState, type ReactNode } from 'react'
import { REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages } from './request-list-model'
import { RequestWorkspaceList, type RequestWorkspaceListProps } from './request-workspace-list'
import {
  REQUEST_ATTENTION_GROUP_LABELS,
  REQUEST_ATTENTION_GROUP_ORDER,
  requestAttentionGroup,
  type RequestAttentionGroup,
} from './request-workspace-model'
import './request-workspace-sidebar.css'

type QueueRow = { item: RequestQueueItemResponse; section: RequestQueueSection }

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
  error,
  actionError,
  maintainer,
  onRetry,
  onLoadMore,
  onAction,
  params,
  pendingId,
  selectedId,
}: Pick<
  RequestWorkspaceListProps,
  'loading' | 'error' | 'onRetry' | 'onAction' | 'pendingId' | 'selectedId'
> & {
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
  const openSearch = useCallback(() => onCollapsedChange(false), [onCollapsedChange])
  const searching = query.trim().length > 0
  const common = { loading, error, onRetry, onAction, params, pendingId, selectedId }
  const rows = (section: RequestQueueSection): QueueRow[] =>
    pages?.[section].requests.map((item) => ({ item, section })) ?? []
  const grouped = groupRows(REQUEST_QUEUE_SECTION_ORDER.flatMap(rows))
  const nextSection = REQUEST_QUEUE_SECTION_ORDER.find((section) => pages?.[section].next_cursor)
  const activeHasMore = Boolean(pages?.active.next_cursor)
  // Active rows page, so loaded lengths are floors until the last page is in.
  const activeCount = (value: number) => `${value}${activeHasMore ? '+' : ''}`
  // Unclaimed and Set aside only ever hold maintainer placements, so readers
  // see just their work and the finished history.
  const disclosures = (
    maintainer ? (['unclaimed', 'set_aside', 'done'] as const) : (['done'] as const)
  ).map((group) => ({ group, section: group }))
  const needsYou = grouped.needs_you
  const waiting = grouped.waiting
  return (
    <aside
      aria-label="Requests workspace"
      className={cn(
        'request-workspace-sidebar',
        collapsed && 'request-workspace-sidebar--collapsed',
      )}
    >
      <div className="request-workspace-collapsed-view">
        <Button
          aria-label="Expand requests sidebar"
          className="text-muted-foreground"
          onClick={openSearch}
          size="icon-sm"
          title="Expand requests sidebar"
          type="button"
          variant="ghost"
        >
          <ChevronRight aria-hidden="true" />
        </Button>
        <RequestWorkspaceSpine grouped={grouped} params={params} selectedId={selectedId} />
      </div>
      <div className="request-workspace-expanded-view">
        <div className="request-workspace-sidebar-tools">
          <NavigationSearch
            clearLabel="Clear request search"
            label="Search requests"
            onChange={onSearch}
            onOpen={openSearch}
            placeholder="Search requests"
            status={loading && searching ? 'Searching requests' : undefined}
            value={query}
          />
          <Button
            aria-label="Collapse requests sidebar"
            className="text-muted-foreground"
            onClick={() => onCollapsedChange(true)}
            size="icon-sm"
            title="Collapse requests sidebar"
            type="button"
            variant="ghost"
          >
            <ChevronLeft aria-hidden="true" />
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
              items={REQUEST_QUEUE_SECTION_ORDER.flatMap(rows)}
              onLoadMore={() => {
                if (nextSection) onLoadMore(nextSection)
              }}
            />
          ) : (
            <>
              {(maintainer || needsYou.length > 0) && (
                <section>
                  <RequestWorkspaceGroupLabel count={activeCount(needsYou.length)} group="needs_you" strong />
                  <RequestWorkspaceList
                    {...common}
                    emptyLabel={EMPTY_LABELS.needs_you}
                    hasMore={activeHasMore && waiting.length === 0}
                    items={needsYou}
                    onLoadMore={() => onLoadMore('active')}
                  />
                </section>
              )}
              {(waiting.length > 0 || (!maintainer && needsYou.length === 0)) && (
                <section>
                  <RequestWorkspaceGroupLabel
                    count={activeCount(waiting.length)}
                    group="waiting"
                    label={maintainer ? undefined : 'Open'}
                  />
                  <RequestWorkspaceList
                    {...common}
                    emptyLabel={maintainer ? EMPTY_LABELS.waiting : 'No open requests.'}
                    hasMore={activeHasMore}
                    items={waiting}
                    onLoadMore={() => onLoadMore('active')}
                  />
                </section>
              )}
              <div className="request-workspace-disclosures">
                {disclosures.map(({ group, section }) => (
                  <RequestWorkspaceDisclosure
                    count={count(pages?.[section])}
                    key={group}
                    label={REQUEST_ATTENTION_GROUP_LABELS[group]}
                    selectedInside={grouped[group].some(({ item }) => item.request.id === selectedId)}
                  >
                    <RequestWorkspaceList
                      {...common}
                      emptyLabel={EMPTY_LABELS[group]}
                      hasMore={Boolean(pages?.[section].next_cursor)}
                      items={grouped[group]}
                      onLoadMore={() => onLoadMore(section)}
                    />
                  </RequestWorkspaceDisclosure>
                ))}
              </div>
            </>
          )}
          {pages && (
            <p className="request-workspace-footer">
              {maintainer ? (
                <>
                  <span className="text-foreground">{summaryCount(needsYou.length, activeHasMore)} you</span>
                  {' · '}
                  {activeCount(waiting.length)} waiting
                </>
              ) : (
                `${activeCount(needsYou.length + waiting.length)} open`
              )}
            </p>
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
    grouped[requestAttentionGroup(row.section, row.item.attention.reason)].push(row)
  }
  return grouped
}

function RequestWorkspaceGroupLabel({
  count,
  group,
  label = REQUEST_ATTENTION_GROUP_LABELS[group],
  strong = false,
}: {
  count: string
  group: RequestAttentionGroup
  label?: string
  strong?: boolean
}) {
  return (
    <h2
      className={cn(
        'request-workspace-group-label',
        strong ? 'text-foreground' : 'text-muted-foreground',
      )}
    >
      <span>{label}</span>
      <span className="tabular-nums">{count}</span>
    </h2>
  )
}

function RequestWorkspaceDisclosure({
  children,
  count,
  label,
  selectedInside,
}: {
  children: ReactNode
  count: string
  label: string
  selectedInside: boolean
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
        <span className="ml-auto flex items-center gap-2">
          {selectedInside && !open && (
            <span
              aria-label="Selected request is in this section"
              className="size-[5px] rounded-full bg-foreground"
            />
          )}
          <span className="tabular-nums">{count}</span>
        </span>
      </button>
      <div hidden={!open} id={id}>
        {children}
      </div>
    </section>
  )
}

/**
 * The collapsed rail. One pip per loaded request so the sidebar stays
 * readable at 54px: ink for rows that need the viewer, grey for the rest,
 * green for finished work.
 */
function RequestWorkspaceSpine({
  grouped,
  params,
  selectedId,
}: {
  grouped: Record<RequestAttentionGroup, QueueRow[]>
  params: RepoParams
  selectedId?: string
}) {
  return (
    <nav aria-label="Loaded requests" className="request-workspace-spine">
      {REQUEST_ATTENTION_GROUP_ORDER.map((group) =>
        grouped[group].length ? (
          <ul aria-label={REQUEST_ATTENTION_GROUP_LABELS[group]} key={group}>
            {grouped[group].map(({ item }) => (
              <li key={item.request.id}>
                <Link
                  aria-current={selectedId === item.request.id ? 'page' : undefined}
                  aria-label={item.request.title}
                  className="request-workspace-pip"
                  data-group={group}
                  params={{ ...params, requestId: item.request.id }}
                  preload="intent"
                  search={{}}
                  title={item.request.title}
                  to="/$owner/$repo/requests/$requestId"
                />
              </li>
            ))}
          </ul>
        ) : null,
      )}
    </nav>
  )
}

function count(page?: RequestQueuePageResponse) {
  return `${page?.requests.length ?? 0}${page?.next_cursor ? '+' : ''}`
}

function summaryCount(value: number, more: boolean) {
  return `${value}${more ? '+' : ''} ${value === 1 && !more ? 'needs' : 'need'}`
}
