import { repoViews } from '@/api/repo-views'
import type { RepoLiveState } from '@/api/types'
import { RepoContentError } from '@/components/repo-content-error'
import { RepoHomePage } from '@/features/repo-detail/repo-home-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import {
  EMPTY_REPO_CODE_LOADER_DATA,
  prepareRepoCodeLoaderData,
  repoCodeCacheKeys,
  useRepoCodeResources,
} from '@/features/repo-detail/use-repo-code-resources'
import { resolveViewingAs } from '@/features/repo-detail/viewing-as'
import { createFileRoute } from '@tanstack/react-router'
import { auth } from '@clerk/tanstack-react-start/server'

const MAIN = { kind: 'main' } as const

export const Route = createFileRoute('/$owner/$repo/_code/')({
  loaderDeps: ({ search }) => ({ view: search.view ?? null }),
  loader: async ({ abortController, deps, params, parentMatchPromise }) => {
    if (typeof window !== 'undefined') return EMPTY_REPO_CODE_LOADER_DATA
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const { userId } = await auth()
    const scope = repoResourceScope(live.repo, userId)
    const view = resolveViewingAs(repoViews(live.repo.views), live.repo.access.view, deps.view)
    return prepareRepoCodeLoaderData({
      file: null,
      keys: repoCodeCacheKeys(live.repo, scope, view, null, MAIN),
      params,
      signal: abortController.signal,
      version: MAIN,
      view,
    })
  },
  errorComponent: RepoContentError,
  pendingComponent: RepositoryCodePending,
  component: RepoHomeRoute,
})

function RepoHomeRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const code = useRepoCodeResources({
    file: null,
    page: Route.useLoaderData(),
    params,
    repo,
    version: MAIN,
  })
  return <RepoHomePage {...code} params={params} repo={repo} />
}
