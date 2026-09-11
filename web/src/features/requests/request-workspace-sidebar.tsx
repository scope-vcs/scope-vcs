import type { RepoParams } from '@/api/types'
import type { RequestQueueSection, RequestQueuePageResponse } from '@/api/types.generated'
import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { ChevronLeft, ChevronRight, Inbox } from 'lucide-react'
import { useCallback, useId, useState, type ReactNode } from 'react'
import { REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages } from './request-list-model'
import { RequestWorkspaceList, type RequestWorkspaceListProps } from './request-workspace-list'
import './request-workspace-sidebar.css'

export function RequestWorkspaceSidebar({
  pages,
  collapsed,
  onCollapsedChange,
  query,
  onSearch,
  loading,
  error,
  actionError,
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
  onLoadMore: (section: RequestQueueSection) => void
  params: RepoParams
}) {
  const openSearch = useCallback(() => onCollapsedChange(false), [onCollapsedChange])
  const searching = query.trim().length > 0
  const common = { loading, error, onRetry, onAction, params, pendingId, selectedId }
  const rows = (section: RequestQueueSection) =>
    pages?.[section].requests.map((item) => ({ item, section })) ?? []
  const nextSection = REQUEST_QUEUE_SECTION_ORDER.find((section) => pages?.[section].next_cursor)
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
        <Inbox aria-hidden="true" className="mt-1 size-4 text-muted-foreground" />
        <span className="font-mono text-[10px] text-success-strong">{count(pages?.active)}</span>
      </div>
      <div className="request-workspace-expanded-view">
        <div className="request-workspace-sidebar-tools">
          <NavigationSearch
            clearLabel="Clear request search"
            label="Search requests"
            onChange={onSearch}
            onOpen={openSearch}
            placeholder="Search requests"
            status={loading ? 'Searching requests' : undefined}
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
          <p className="px-3 py-2 text-[11px] text-danger-strong" role="alert">
            {actionError}
          </p>
        )}
        <div aria-busy={loading} className="request-workspace-scroll">
          <RequestWorkspaceList
            {...common}
            emptyLabel={searching ? 'No matching requests.' : 'You’re caught up.'}
            hasMore={searching ? Boolean(nextSection) : Boolean(pages?.active.next_cursor)}
            items={searching ? REQUEST_QUEUE_SECTION_ORDER.flatMap(rows) : rows('active')}
            onLoadMore={() => {
              if (!searching) onLoadMore('active')
              else if (nextSection) onLoadMore(nextSection)
            }}
          />
          <div className="mt-3 border-t border-border/55 pt-2.5" hidden={searching}>
            {(
              [
                {
                  section: 'unclaimed',
                  label: 'Unclaimed',
                  empty: 'Every request has a maintainer.',
                },
                { section: 'set_aside', label: 'Set aside', empty: 'Nothing set aside.' },
              ] as const
            ).map(({ section, label, empty }) => (
              <RequestWorkspaceDisclosure
                key={section}
                label={label}
                page={pages?.[section]}
                selectedId={selectedId}
              >
                <RequestWorkspaceList
                  {...common}
                  emptyLabel={empty}
                  hasMore={Boolean(pages?.[section].next_cursor)}
                  items={rows(section)}
                  onLoadMore={() => onLoadMore(section)}
                />
              </RequestWorkspaceDisclosure>
            ))}
          </div>
        </div>
      </div>
    </aside>
  )
}

function RequestWorkspaceDisclosure({
  children,
  label,
  page,
  selectedId,
}: {
  children: ReactNode
  label: string
  page: RequestQueuePageResponse | undefined
  selectedId?: string
}) {
  const [open, setOpen] = useState(false)
  const id = useId()
  const selectedInside = page?.requests.some((item) => item.request.id === selectedId)
  return (
    <section className="mb-0.5">
      <button
        aria-controls={id}
        aria-expanded={open}
        className="flex min-h-9 w-full items-center gap-2 rounded-md px-3 py-2 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/55 hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring aria-expanded:text-foreground"
        onClick={() => setOpen(!open)}
        type="button"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn(
            'size-[13px] transition-transform motion-reduce:transition-none',
            open && 'rotate-90',
          )}
        />
        <span>{label}</span>
        <span className="ml-auto flex items-center gap-2">
          {selectedInside && !open && (
            <span
              aria-label="Selected request is in this section"
              className="size-[5px] rounded-full bg-success"
            />
          )}
          <span className="text-[11px] font-normal tabular-nums">{count(page)}</span>
        </span>
      </button>
      <div className="pt-1 pb-2" hidden={!open} id={id}>
        {children}
      </div>
    </section>
  )
}

function count(page?: RequestQueuePageResponse) {
  return `${page?.requests.length ?? 0}${page?.next_cursor ? '+' : ''}`
}
