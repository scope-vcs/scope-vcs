import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { repoViews } from '@/api/repo-views'
import type { RepoLiveState } from '@/api/types'
import { UpdatePage } from '@/features/history/update-page'
import { UpdatePagePending } from '@/features/history/update-page-pending'
import { parseUpdateSearch } from '@/features/history/update-search'
import { resolveViewingAs } from '@/features/repo-detail/viewing-as'
import { loadHistoryEntry } from '@/routes/-repo-history-actions'
import { RouteErrorContent } from '@/components/route-error-page'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/updates/$entryId')({
  validateSearch: parseUpdateSearch,
  loaderDeps: ({ search }) => ({ view: search.view ?? null }),
  staleTime: Infinity,
  loader: async ({ deps, params, parentMatchPromise }) => {
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const view = resolveViewingAs(repoViews(live.repo.views), live.repo.access.view, deps.view)
    const loaded = await loadHistoryEntry({
      data: { owner: params.owner, repo: params.repo, view, entry: params.entryId },
    })
    return {
      initialEntry: loaded.entry,
      initialEntryScope: repoResourceScope(live.repo, loaded.viewerId),
    }
  },
  errorComponent: ({ error }) => (
    <RouteErrorContent
      error={error}
      fallbackMessage="Unexpected update error"
      title="Update unavailable"
    />
  ),
  pendingComponent: UpdatePagePending,
  component: UpdateRoute,
})

function UpdateRoute() {
  const { initialEntry, initialEntryScope } = Route.useLoaderData()
  return (
    <UpdatePage
      initialEntry={initialEntry}
      initialEntryScope={initialEntryScope}
      params={Route.useParams()}
      search={Route.useSearch()}
    />
  )
}
