import { repoViews } from '@/api/repo-views'
import type { RepoLiveState } from '@/api/types'
import { RepoContentError } from '@/components/repo-content-error'
import { RepoCodePage } from '@/features/repo-detail/repo-code-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import {
  parseRepositoryVersionSearch,
  repositoryVersion,
  type RepositoryVersionSearch,
} from '@/features/repo-detail/repository-version'
import {
  EMPTY_REPO_CODE_LOADER_DATA,
  prepareRepoCodeLoaderData,
  repoCodeCacheKeys,
  useRepoCodeResources,
} from '@/features/repo-detail/use-repo-code-resources'
import { resolveViewingAs } from '@/features/repo-detail/viewing-as'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { auth } from '@clerk/tanstack-react-start/server'
import { useCallback, useMemo } from 'react'

type RepoCodeSearch = RepositoryVersionSearch & { file?: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { ...parseRepositoryVersionSearch(search), file: parseRouteFileSearch(search.file) }
}

export const Route = createFileRoute('/$owner/$repo/_code/code')({
  validateSearch: parseRepoCodeSearch,
  loaderDeps: ({ search }) => ({
    request: search.request,
    update: search.update,
    view: search.view ?? null,
  }),
  loader: async ({ abortController, deps, location, params, parentMatchPromise }) => {
    if (typeof window !== 'undefined') return EMPTY_REPO_CODE_LOADER_DATA
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const { userId } = await auth()
    const scope = repoResourceScope(live.repo, userId)
    const view = resolveViewingAs(repoViews(live.repo.views), live.repo.access.view, deps.view)
    const version = repositoryVersion(deps)
    const file = parseRepoCodeSearch(location.search).file ?? null
    return prepareRepoCodeLoaderData({
      file,
      keys: repoCodeCacheKeys(live.repo, scope, view, file, version),
      params,
      signal: abortController.signal,
      version,
      view,
    })
  },
  errorComponent: RepoContentError,
  pendingComponent: RepositoryCodePending,
  component: RepoCodeRoute,
})

function RepoCodeRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const { request, update } = search
  const version = useMemo(() => repositoryVersion({ request, update }), [request, update])
  const code = useRepoCodeResources({
    file: search.file ?? null,
    page: Route.useLoaderData(),
    params,
    repo,
    version,
  })
  const selectedPath = code.selectedPath
  const selectFile = useCallback((path: string) => {
    const nextPath = displayRouteFilePath(path)
    if (nextPath === selectedPath) return
    void navigate({
      resetScroll: false,
      search: (current) => ({ ...current, file: nextPath }),
    })
  }, [navigate, selectedPath])

  return (
    <RepoCodePage
      {...code}
      onSelectFilePath={selectFile}
      params={params}
      repo={repo}
      version={version}
    />
  )
}
