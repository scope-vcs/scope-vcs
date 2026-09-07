import type { RequestQueueSection } from '@/api/request-queue-input'
import type {
  RepoParams,
  RequestList,
  RequestListItem,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/empty-state'
import { PageContent } from '@/components/page-header'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import {
  CheckCircle2,
  ChevronRight,
  GitPullRequest,
  Search,
  UserRound,
} from 'lucide-react'
import { type FormEvent, useEffect, useReducer } from 'react'
import {
  requestQueueViewReducer,
  requestCountLabel,
  REQUEST_QUEUE_SECTION_ORDER,
  type RequestQueuePages,
} from './request-list-model'
import {
  requestAudienceLabel,
  requestAuthorRoleLabel,
  requestMergeabilityLabel,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'
import { AbsoluteTimestamp } from '@/components/timestamp'

import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { restoreRequestQueue, retainRequestQueue } from './request-queue-cache'

const SECTION_DETAILS = {
  your_work: {
    empty: 'Nothing here involves you yet.',
    icon: UserRound,
    title: 'your work',
  },
  open: {
    empty: 'No open requests.',
    icon: GitPullRequest,
    title: 'open',
  },
  closed: {
    empty: 'No closed requests.',
    icon: CheckCircle2,
    title: 'closed',
  },
} as const

export function RequestsPage(props: RequestsPageProps) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const cacheKey = isLoaded ? `${repoResourceScope(repo, userId ?? null)}\0${repo.change_version}` : null
  return <RequestsPageContent initialPages={props.initialPages} loadPage={props.loadPage} params={props.params} key={cacheKey ?? 'pending'} cacheKey={cacheKey} />
}

type RequestsPageProps = {
  initialPages: RequestQueuePages
  loadPage: (section: RequestQueueSection, cursor: string | null, search: string | null) => Promise<RequestList>
  params: RepoParams
}

function RequestsPageContent({
  cacheKey,
  initialPages,
  loadPage,
  params,
}: RequestsPageProps & { cacheKey: string | null }) {
  const { isSignedIn } = useAuth()
  const [state, dispatch] = useReducer(
    requestQueueViewReducer,
    initialPages,
    (pages) => restoreRequestQueue(cacheKey, pages),
  )
  if (state.snapshot !== initialPages) {
    dispatch({ type: 'loader_snapshot_received', pages: initialPages })
  }
  useEffect(() => retainRequestQueue(cacheKey, state), [cacheKey, state])
  const {
    generation,
    loadingSection,
    pages,
    searchDraft,
    searchError,
    searching,
    searchQuery,
    sectionErrors,
  } = state

  async function loadMore(section: RequestQueueSection) {
    const cursor = pages[section].next_cursor
    if (!cursor || loadingSection || searching) return

    const operationGeneration = generation
    dispatch({
      type: 'load_started',
      generation: operationGeneration,
      section,
    })
    try {
      const page = await loadPage(
        section,
        cursor,
        section === 'your_work' ? null : searchQuery || null,
      )
      dispatch({
        type: 'load_succeeded',
        generation: operationGeneration,
        section,
        page,
      })
    } catch (error) {
      dispatch({
        type: 'load_failed',
        generation: operationGeneration,
        section,
        error: errorMessage(
          error,
          `Could not load more ${SECTION_DETAILS[section].title.toLowerCase()} requests.`,
        ),
      })
    }
  }

  async function searchQueue(query: string) {
    if (searching || loadingSection) return
    const normalizedQuery = query.trim()
    if (normalizedQuery === searchQuery) return

    const operationGeneration = generation
    dispatch({ type: 'search_started', generation: operationGeneration })
    try {
      const [open, closed] = await Promise.all([
        loadPage('open', null, normalizedQuery || null),
        loadPage('closed', null, normalizedQuery || null),
      ])
      dispatch({
        type: 'search_succeeded',
        generation: operationGeneration,
        query: normalizedQuery,
        open,
        closed,
      })
    } catch (error) {
      dispatch({
        type: 'search_failed',
        generation: operationGeneration,
        error: errorMessage(error, 'Could not search requests.'),
      })
    }
  }

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    void searchQueue(searchDraft)
  }

  function clearSearch() {
    dispatch({ type: 'search_draft_changed', value: '' })
    void searchQueue('')
  }

  return (
    <PageContent className="pb-16">
      <h1 className="sr-only">Requests</h1>
      <QueueSearch
        busy={Boolean(loadingSection) || searching}
        error={searchError}
        onChange={(value) => dispatch({ type: 'search_draft_changed', value })}
        onClear={clearSearch}
        onSubmit={submitSearch}
        query={searchDraft}
        searching={searching}
        searchQuery={searchQuery}
      />
      <div aria-busy={searching} className="mt-10 grid gap-12">
        {REQUEST_QUEUE_SECTION_ORDER.map((section) => section === 'your_work' && !isSignedIn ? null : (
          <QueueSection
            busy={Boolean(loadingSection) || searching}
            error={sectionErrors[section]}
            key={section}
            loading={loadingSection === section}
            onLoadMore={() => void loadMore(section)}
            page={pages[section]}
            params={params}
            searchQuery={section === 'your_work' ? '' : searchQuery}
            section={section}
          />
        ))}
      </div>
    </PageContent>
  )
}

function QueueSearch({
  busy,
  error,
  onChange,
  onClear,
  onSubmit,
  query,
  searching,
  searchQuery,
}: {
  busy: boolean
  error: string | null
  onChange: (value: string) => void
  onClear: () => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  query: string
  searching: boolean
  searchQuery: string
}) {
  return (
    <form
      className="flex flex-wrap items-center gap-2"
      onSubmit={onSubmit}
      role="search"
    >
      <label className="relative block min-w-0 flex-1 sm:max-w-lg">
        <span className="sr-only">Search open and closed requests</span>
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <input
          className="h-10 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          disabled={busy}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Search requests"
          type="search"
          value={query}
        />
      </label>
      <div className="flex items-center gap-2">
        <Button disabled={busy} size="sm" type="submit" variant="secondary">
          {searching ? 'Searching…' : 'Search'}
        </Button>
        {searchQuery ? (
          <Button
            disabled={busy}
            onClick={onClear}
            size="sm"
            type="button"
            variant="ghost"
          >
            Clear
          </Button>
        ) : null}
      </div>
      {error ? (
        <p className="text-sm text-destructive sm:ml-2" role="alert">
          {error}
        </p>
      ) : null}
      {searching ? <output className="sr-only">Searching requests…</output> : null}
    </form>
  )
}

function QueueSection({
  busy,
  error,
  loading,
  onLoadMore,
  page,
  params,
  searchQuery,
  section,
}: {
  busy: boolean
  error?: string
  loading: boolean
  onLoadMore: () => void
  page: RequestList
  params: RepoParams
  searchQuery: string
  section: RequestQueueSection
}) {
  const details = SECTION_DETAILS[section]
  const Icon = details.icon
  const headingId = `request-queue-${section}`
  const emptyMessage = searchQuery
    ? `Nothing matches “${searchQuery}”.`
    : details.empty

  const Container = section === 'closed' ? 'details' : 'section'
  const Heading = section === 'closed' ? 'summary' : 'div'

  return (
    <Container aria-labelledby={headingId} className="group/section" open={section === 'closed' && searchQuery ? true : undefined}>
      <Heading className="flex items-center gap-2 [&:is(summary)]:cursor-pointer">
        {section === 'closed' ? (
          <ChevronRight aria-hidden="true" className="size-4 text-muted-foreground group-open/section:rotate-90" />
        ) : (
          <Icon aria-hidden="true" className="size-4 text-muted-foreground" />
        )}
        <h2 className="text-sm font-semibold" id={headingId}>
          {details.title}
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {requestCountLabel(page.requests.length, Boolean(page.next_cursor))}
        </span>
      </Heading>

      {page.requests.length ? (
        <div className="mt-2 divide-y divide-border">
          {page.requests.map((request) => (
            <RequestQueueRow
              key={request.id}
              params={params}
              request={request}
              section={section}
            />
          ))}
        </div>
      ) : (
        <EmptyState className="mt-3" inline title={emptyMessage} />
      )}

      {page.next_cursor ? (
        <div className="pt-4">
          <Button
            disabled={busy}
            onClick={onLoadMore}
            size="sm"
            type="button"
            variant="secondary"
          >
            {loading ? 'Loading…' : 'Load more'}
          </Button>
          {loading ? (
            <output className="sr-only">
              Loading more {details.title.toLowerCase()} requests…
            </output>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <p className="mt-2 text-sm text-danger-strong" role="alert">
          {error}
        </p>
      ) : null}
    </Container>
  )
}

function RequestQueueRow({
  params,
  request,
  section,
}: {
  params: RepoParams
  request: RequestListItem
  section: RequestQueueSection
}) {
  return (
    <Link
      className="group block min-w-0 rounded-md py-3 outline-none transition-colors [contain-intrinsic-size:auto_64px] [content-visibility:auto] hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      params={{ ...params, requestId: request.id }}
      title={request.id}
      to="/$owner/$repo/requests/$requestId"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
        <h3 className="break-words text-base font-medium leading-6 group-hover:underline">
          {request.title}
        </h3>
        <Badge className="font-medium text-foreground" variant={section === 'open' ? 'neutral' : requestStatusTone(request)}>
          {section === 'open' ? requestMergeabilityLabel(request) : requestStatusLabel(request)}
        </Badge>
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] leading-5 text-muted-foreground">
        <QueueDate request={request} section={section} />
        <span aria-hidden="true">·</span>
        <span>{requestAuthorRoleLabel(request)}</span>
        <span aria-hidden="true">·</span>
        <span>{requestAudienceLabel(request)}</span>
      </div>
      {request.title !== request.name ? (
        <div className="mt-1 break-all font-mono text-[13px] text-muted-foreground">
          {request.name}
        </div>
      ) : null}
    </Link>
  )
}

function QueueDate({
  request,
  section,
}: {
  request: RequestListItem
  section: RequestQueueSection
}) {
  if (section === 'open' && request.submitted_at_unix !== null) {
    return (
      <AbsoluteTimestamp
        className="tabular-nums"
        compact
        prefix="Submitted "
        value={request.submitted_at_unix}
      />
    )
  }
  return (
    <AbsoluteTimestamp
      className="tabular-nums"
      compact
      prefix="Updated "
      value={request.updated_at_unix}
    />
  )
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback
}
