import { RepoShell } from '@/components/repo-shell'
import { RepositoryRoutePending } from '@/components/repository-route-pending'
import { RepositoryHtmlPreviewProvider } from '@/components/repository-html-preview-store'
import { RouteErrorPage } from '@/components/route-error-page'
import { RouteNotFoundPage } from '@/components/route-not-found-page'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { useRepoLiveRefresh } from '@/features/repo-detail/repo-live-refresh'
import { loadRepoSummary, repoSummaryIdentity } from '@/features/repo-detail/repo-summary-resource'
import { useRepoSummary } from '@/features/repo-detail/use-repo-summary'
import { parseViewingAsSearch } from '@/features/repo-detail/viewing-as'
import { loadRepoSummaryState } from '@/routes/-repo-summary-actions'
import {
  Outlet,
  createFileRoute,
  retainSearchParams,
  useRouter,
} from '@tanstack/react-router'
import { useCallback } from 'react'

export const Route = createFileRoute('/$owner/$repo')({
  validateSearch: parseViewingAsSearch,
  search: { middlewares: [retainSearchParams(['view'])] },
  ssr: false,
  loader: ({ params }) => loadRepoSummary(
    repoSummaryIdentity(params),
    (signal) => loadRepoSummaryState(params, signal),
  ),
  errorComponent: RepoLayoutError,
  notFoundComponent: RouteNotFoundPage,
  pendingComponent: RepositoryRoutePending,
  component: RepoLayoutRoute,
})

function RepoLayoutError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected repository error"
      title="Repository unavailable"
    />
  )
}

function RepoLayoutRoute() {
  const params = Route.useParams()
  const { owner, repo } = params
  const router = useRouter()
  const identity = repoSummaryIdentity({ owner, repo })
  const loaded = Route.useLoaderData()
  const live = useRepoSummary({
    identity,
    load: useCallback((signal: AbortSignal) => loadRepoSummaryState({ owner, repo }, signal), [owner, repo]),
    onRejected: useCallback(() => void router.invalidate(), [router]),
  }) ?? loaded
  const subscribe = useRepoLiveRefresh(live)
  return (
    <RepoLayoutProvider live={live} subscribe={subscribe} summaryIdentity={identity}>
      <RepositoryHtmlPreviewProvider>
        <RepoShell params={params} repo={live.repo}>
          <Outlet />
        </RepoShell>
      </RepositoryHtmlPreviewProvider>
    </RepoLayoutProvider>
  )
}
