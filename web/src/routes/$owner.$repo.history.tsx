import { parseHistoryFeed, parseVisibilityChange } from '@/api/history-inputs'
import { HistoryPagePending } from '@/features/history/history-page-pending'
import { HistoryPage, type HistorySearch } from '@/features/history/history-page'
import { parseRouteFilePathSearch } from '@/lib/route-file'
import { loadHistoryEntry, loadHistoryPage } from '@/routes/-repo-history-actions'
import { RouteErrorContent } from '@/components/route-error-page'
import { createFileRoute, redirect } from '@tanstack/react-router'
import type { ProjectionPreviewAudience } from '@/api/types.generated'

export const Route = createFileRoute('/$owner/$repo/history')({
  validateSearch: parseHistorySearch,
  loaderDeps: ({ search }) => ({ audience: search.audience ?? null, feed: search.feed ?? 'updates' }),
  staleTime: Infinity,
  loader: async ({ deps, params, location }) => {
    const search = parseHistorySearch(location.search)
    const [page, initialEntry] = await Promise.all([
      loadHistoryPage({ data: { ...params, audience: deps.audience, feed: deps.feed, before: null } }),
      search.entry
        ? loadHistoryEntry({ data: { ...params, audience: deps.audience, entry: search.entry } })
        : Promise.resolve(null),
    ])
    if (deps.feed === 'updates' && initialEntry?.kind === 'visibility_change') {
      throw redirect({
        to: '/$owner/$repo/history',
        params,
        search: { ...search, feed: 'all' },
        replace: true,
      })
    }
    return { page, initialEntry }
  },
  errorComponent: ({ error }) => (
    <RouteErrorContent
      error={error}
      fallbackMessage="Unexpected history error"
      title="History unavailable"
    />
  ),
  pendingComponent: HistoryPagePending,
  component: HistoryRoute,
})

function HistoryRoute() {
  const { page, initialEntry } = Route.useLoaderData()
  return (
    <HistoryPage
      initialPage={page}
      initialEntry={initialEntry}
      params={Route.useParams()}
      search={Route.useSearch()}
    />
  )
}

function parseHistorySearch(search: Record<string, unknown>): HistorySearch {
  return {
    audience: searchHistoryAudience(search.audience),
    feed: parseHistoryFeed(search.feed),
    visibility_change: parseVisibilityChange(search.visibility_change) ?? undefined,
    entry: searchHistoryEntryId(search.entry),
    path: parseRouteFilePathSearch(search.path),
  }
}

function searchHistoryAudience(value: unknown): ProjectionPreviewAudience | undefined {
  if (value === undefined || value === null || value === '') {
    return undefined
  }
  if (value === 'private' || value === 'public') {
    return value
  }
  throw new Error(`Unsupported history audience: ${String(value)}`)
}

function searchHistoryEntryId(value: unknown) {
  if (typeof value === 'string') {
    const entryId = value.trim()
    return entryId ? entryId : undefined
  }

  if (typeof value === 'number' && Number.isFinite(value)) {
    return String(value)
  }

  return undefined
}
