import { parseHistoryFeed, parseVisibilityChange } from '@/api/history-inputs'
import type { ProjectionPreviewAudience } from '@/api/types'
import { HistoryError } from '@/features/history/history-error'
import { HistoryPagePending } from '@/features/history/history-page-pending'
import { HistoryPage } from '@/features/history/history-page'
import { parseRouteFileSearch } from '@/lib/route-file'
import { loadHistoryEntry, loadHistoryPage } from '@/routes/-repo-history-actions'
import { createFileRoute, redirect } from '@tanstack/react-router'

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
  errorComponent: HistoryError,
  pendingComponent: HistoryPagePending,
  component: HistoryRoute,
})

function HistoryRoute() {
  const { page, initialEntry } = Route.useLoaderData()
  return (
    <HistoryPage
      initialPage={page}
      initialEntry={initialEntry}
      key={`${page.repo_id}:${page.audience}:${page.feed}:${page.generation}`}
      params={Route.useParams()}
      search={Route.useSearch()}
    />
  )
}

export type HistorySearch = {
  audience?: ProjectionPreviewAudience
  feed?: 'updates' | 'all'
  visibility_change?: string
  entry?: string
  path?: string
}

function parseHistorySearch(search: Record<string, unknown>): HistorySearch {
  return {
    audience: searchHistoryAudience(search.audience),
    feed: parseHistoryFeed(search.feed),
    visibility_change: parseVisibilityChange(search.visibility_change) ?? undefined,
    entry: searchHistoryEntryId(search.entry),
    path: searchHistoryPath(search.path),
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

function searchHistoryPath(value: unknown) {
  const path = parseRouteFileSearch(value)
  return path ? `/${path}` : undefined
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
