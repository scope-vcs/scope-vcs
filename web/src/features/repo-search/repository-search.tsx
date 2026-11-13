import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { DiffStatSummary } from '@/components/diff-stat'
import { NavigationSearch } from '@/components/navigation-search'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { diffStatWithChanges } from '@/lib/diff-stat'
import { resourceErrorMessage, useCachedResource } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { loadRepoContent } from '@/routes/-repo-code-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link, useLocation, useSearch } from '@tanstack/react-router'
import { Eye, File, GitBranch, GitPullRequest, History } from 'lucide-react'
import { useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { repoContentResource } from '../repo-detail/repo-content-cache'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import {
  parseRepositoryVersionSearch,
  repositoryVersion,
  repositoryVersionSearch,
} from '../repo-detail/repository-version'
import { repoCodeCacheKeys, repoCodeRefreshVersion } from '../repo-detail/use-repo-code-resources'
import { useViewingAs } from '../repo-detail/use-viewing-as'
import { actorHandle } from '../requests/request-actor'
import {
  historyEntryOpensCode,
  repositorySearchFeed,
  repositorySearchGroups,
  type RepositorySearchFilter,
  type RepositorySearchRow,
} from './repository-search-model'
import { REPOSITORY_SEARCH_INPUT_ID, useRepositorySearch } from './repository-search-context'
import { useRepositorySearchResults } from './use-repository-search-results'

const FILTERS: { label: string; value: RepositorySearchFilter }[] = [
  { label: 'All', value: 'all' },
  { label: 'Requests', value: 'requests' },
  { label: 'Pushes & merges', value: 'updates' },
  { label: 'Visibility', value: 'visibility' },
  { label: 'Files', value: 'files' },
]
const EMPTY_SEARCH_RESULTS = { history: [], requests: [] }
const SEARCH_DELAY_MS = 200
const PAGE_SEARCH_ROUTE = /\/(?:updates\/|requests\/[^/]+\/changes(?:\/|$))/

export function RepositorySearch({ params, repo }: { params: RepoParams; repo: RepoSummaryResponse }) {
  const { isLoaded, userId } = useAuth()
  const { view } = useViewingAs()
  const pathname = useLocation({ select: (location) => location.pathname })
  const routeSearch = useSearch({ strict: false })
  const version = useMemo(
    () => repositoryVersion(parseRepositoryVersionSearch(routeSearch)),
    [routeSearch],
  )
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [settledQuery, setSettledQuery] = useState('')
  const { filter, setFilter } = useRepositorySearch()
  const [activeIndex, setActiveIndex] = useState(0)
  const [more, setMore] = useState<{ error: string | null; loading: boolean }>({ error: null, loading: false })
  const rootRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const listId = useId()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const search = query.trim()

  useEffect(() => {
    const timer = setTimeout(() => setSettledQuery(search), SEARCH_DELAY_MS)
    return () => clearTimeout(timer)
  }, [search])

  const feed = repositorySearchFeed(filter)
  const found = useRepositorySearchResults({
    enabled: open,
    feed,
    params,
    query: settledQuery,
    repo,
    scope,
    view,
  })
  const { owner, repo: repoName } = params
  const loadFiles = useCallback(
    (signal: AbortSignal) => loadRepoContent({
      data: { owner, repo: repoName, view, ...repositoryVersionSearch(version) },
      signal,
    }),
    [owner, repoName, version, view],
  )
  const files = useCachedResource({
    enabled: open && repo.lifecycle_state === 'Ready',
    fallbackError: 'Repository files are unavailable.',
    identity: repoCodeCacheKeys(repo, scope, view, null, version).contentIdentity,
    load: loadFiles,
    resource: repoContentResource,
    version: repoCodeRefreshVersion(repo, version),
  })
  const queryPending = search !== settledQuery
  const groups = useMemo(() => repositorySearchGroups({
    files: files.value?.files ?? [],
    filter,
    query: search,
    results: queryPending ? EMPTY_SEARCH_RESULTS : found.results,
    version,
  }), [files.value, filter, found.results, queryPending, search, version])
  const rows = groups.flatMap((group) => group.rows)
  const active = Math.min(activeIndex, Math.max(0, rows.length - 1))
  const activeRow = rows[active]

  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [active])

  function close() {
    setOpen(false)
    setActiveIndex(0)
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (!rows.length) return
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      setActiveIndex((active + (event.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length)
    } else if (event.key === 'Enter') {
      event.preventDefault()
      if (queryPending && activeRow?.kind !== 'file') return
      listRef.current?.querySelector<HTMLElement>('[data-active="true"]')?.click()
    }
  }

  const settled = found.loaded && !queryPending
  const failure = found.failure ?? (files.error ? files : null)
  const pagingRequests = filter === 'requests'
  const canLoadMore = !queryPending && (pagingRequests
    ? found.hasMoreRequests
    : feed !== 'all' && found.hasMoreHistory)

  async function loadMore() {
    setMore({ error: null, loading: true })
    try {
      if (pagingRequests) await found.loadMoreRequests()
      else await found.loadOlderHistory()
      setMore({ error: null, loading: false })
    } catch (error) {
      setMore({
        error: resourceErrorMessage(error, pagingRequests ? 'More requests are unavailable.' : 'Older history is unavailable.'),
        loading: false,
      })
    }
  }

  return (
    <div
      className="relative min-w-0"
      onBlur={(event) => {
        if (!rootRef.current?.contains(event.relatedTarget)) close()
      }}
      ref={rootRef}
    >
      <NavigationSearch
        clearLabel="Clear search"
        input={{
          'aria-activedescendant': open && activeRow ? `${listId}-${activeRow.id}` : undefined,
          'aria-autocomplete': 'list',
          'aria-controls': listId,
          'aria-expanded': open,
          id: REPOSITORY_SEARCH_INPUT_ID,
          onClick: () => setOpen(true),
          onFocus: () => setOpen(true),
          role: 'combobox',
        }}
        label="Search requests, history and files"
        onChange={(value) => {
          setQuery(value)
          setActiveIndex(0)
          setOpen(true)
        }}
        onEscape={close}
        onKeyDown={onKeyDown}
        onOpen={() => setOpen(true)}
        placeholder="Search requests, history and files…"
        shortcut={!PAGE_SEARCH_ROUTE.test(pathname)}
        value={query}
      />
      {open && (
        <div className="absolute inset-x-0 top-full z-50 mt-1.5 overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-[var(--shadow-pop)]">
          <div className="overflow-x-auto p-2">
            <ToggleGroup
              aria-label="Search filter"
              onValueChange={(value) => {
                if (value) {
                  setFilter(value as RepositorySearchFilter)
                  setActiveIndex(0)
                }
              }}
              type="single"
              value={filter}
            >
              {FILTERS.map((option) => (
                <ToggleGroupItem key={option.value} value={option.value}>{option.label}</ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
          <div
            aria-label="Search results"
            className="max-h-[min(30rem,60dvh)] overflow-y-auto border-t border-border"
            id={listId}
            ref={listRef}
            role="listbox"
          >
            {groups.map((group) => (
              <div aria-labelledby={`${listId}-${group.id}`} key={group.id} role="group">
                <div
                  className="flex justify-between px-3 pt-2 pb-1 text-xs font-semibold text-muted-foreground"
                  id={`${listId}-${group.id}`}
                >
                  <span>{group.label}</span>
                  {search ? <span className="font-normal tabular-nums">{group.total}</span> : null}
                </div>
                {group.rows.map((row) => {
                  const rowActive = activeRow?.id === row.id
                  return (
                    <SearchRow
                      active={rowActive}
                      file={typeof routeSearch.file === 'string' ? routeSearch.file : undefined}
                      id={`${listId}-${row.id}`}
                      key={row.id}
                      onNavigate={close}
                      onPoint={() => setActiveIndex(rows.indexOf(row))}
                      params={params}
                      row={row}
                      version={repositoryVersionSearch(version)}
                    />
                  )
                })}
              </div>
            ))}
            {failure ? (
              <div className="flex items-center justify-between gap-3 px-3 py-3 text-xs text-danger-strong" role="alert">
                <span>{failure.error}</span>
                <Button onClick={failure.retry} size="sm" type="button" variant="secondary">Retry</Button>
              </div>
            ) : !groups.length && (
              <output className="block px-3 py-4 text-xs text-muted-foreground">
                {settled ? 'No matches.' : 'Searching…'}
              </output>
            )}
            {canLoadMore && (
              <div className="flex items-center gap-3 px-3 py-2 text-xs">
                <Button disabled={more.loading} onClick={() => void loadMore()} size="sm" type="button" variant="secondary">
                  {pagingRequests ? 'Load more' : 'Load older'}
                </Button>
                {more.error && <span className="text-danger-strong" role="alert">{more.error}</span>}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function SearchRow({
  active,
  file,
  id,
  onNavigate,
  onPoint,
  params,
  row,
  version,
}: {
  active: boolean
  file: string | undefined
  id: string
  onNavigate: () => void
  onPoint: () => void
  params: RepoParams
  row: RepositorySearchRow
  version: ReturnType<typeof repositoryVersionSearch>
}) {
  const shared = {
    'aria-selected': active,
    className: cn(
      'grid grid-cols-[1rem_minmax(0,1fr)_auto] items-start gap-x-2 px-3 py-1.5 text-[13px] leading-[18px] hover:bg-muted',
      active && 'bg-muted',
    ),
    'data-active': active,
    id,
    onClick: onNavigate,
    onMouseDown: (event: { preventDefault: () => void }) => event.preventDefault(),
    onMouseMove: onPoint,
    role: 'option',
  }
  if (row.kind === 'main') {
    return (
      <Link {...shared} params={params} search={{ file }} to="/$owner/$repo/code">
        <RowBody icon={<GitBranch />} title="main">Latest code on main</RowBody>
      </Link>
    )
  }
  if (row.kind === 'request') {
    const { author, request } = row.item
    const changes = diffStatWithChanges(request.diff_stat)
    return (
      <Link {...shared} params={params} search={{ file, request: request.id }} to="/$owner/$repo/code">
        <RowBody
          icon={<GitPullRequest />}
          meta={changes ? <DiffStatSummary detail="exact" stat={changes} /> : undefined}
          title={request.title}
        >
          <span className="font-mono">{request.name}</span> · {actorHandle(author)}
        </RowBody>
      </Link>
    )
  }
  if (row.kind === 'file') {
    return (
      <Link {...shared} params={params} search={{ ...version, file: row.path }} to="/$owner/$repo/code">
        <RowBody icon={<File />} mono title={row.path} />
      </Link>
    )
  }
  const { entry } = row
  const [firstLine = ''] = entry.message.split('\n', 1)
  const title = firstLine === '' ? 'Repository updated' : firstLine
  const body = (
    <RowBody icon={historyEntryOpensCode(entry) ? <History /> : <Eye />} title={title}>
      {historyKindLabel(entry.kind)}
      {entry.author ? ` · ${entry.author}` : ''}
      {entry.occurred_at_unix !== null && <> · <RelativeTimestamp value={entry.occurred_at_unix} /></>}
    </RowBody>
  )
  return historyEntryOpensCode(entry) ? (
    <Link {...shared} params={params} search={{ file, update: entry.source_id }} to="/$owner/$repo/code">
      {body}
    </Link>
  ) : (
    <Link {...shared} params={{ ...params, entryId: entry.source_id }} search={{}} to="/$owner/$repo/updates/$entryId">
      {body}
    </Link>
  )
}

function RowBody({
  children,
  icon,
  meta,
  mono,
  title,
}: {
  children?: ReactNode
  icon: ReactNode
  meta?: ReactNode
  mono?: boolean
  title: string
}) {
  return (
    <>
      <span aria-hidden="true" className="mt-0.5 text-muted-foreground [&_svg]:size-3.5">{icon}</span>
      <span className={cn('min-w-0 truncate font-medium', mono && 'font-mono text-xs font-normal')}>{title}</span>
      {meta ? <span className="text-xs">{meta}</span> : <span />}
      {children && <span className="col-start-2 col-end-4 min-w-0 truncate text-xs text-muted-foreground">{children}</span>}
    </>
  )
}

function historyKindLabel(kind: 'push' | 'merged_request' | 'visibility_change' | 'views_change') {
  if (kind === 'push') return 'Push'
  if (kind === 'merged_request') return 'Merge'
  return kind === 'views_change' ? 'Views' : 'Visibility'
}
