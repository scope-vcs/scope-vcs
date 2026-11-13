import {
  createApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { arrayOf, stripTrailingSlash } from './http'
import { repoRoute } from './paths'
import type { RepositoryVersionSearch } from '../features/repo-detail/repository-version'
import type { RepoContent, RepoLiveState, RepoParams } from './types'
import type { RepoSummaryResponse, RepositoryDependencyCheckResponse, ViewId } from './types.generated'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  RepoFileContentResponseValidator,
  RepoFileResponseValidator,
  RepoSummaryResponseValidator,
  RepositoryDependencyCheckResponseValidator,
} from './validators.generated'

export async function loadRepoContentForRequest(
  data: RepoParams & { view: ViewId } & RepositoryVersionSearch,
  signal?: AbortSignal,
) {
  const api = createApiClient()
  const files = await api.get(
    `${repoRoute(ApiRouteTemplates.repoFiles, data)}?${repoVersionQuery(data)}`,
    arrayOf(RepoFileResponseValidator),
    { auth: 'optional', signal },
  )

  const publicApi = stripTrailingSlash(getPublicApiConnection('building clone command'))
  const gitPath = buildApiPath(ApiRouteTemplates.gitRepo, {
    view: data.view,
    org: data.owner,
    repo: data.repo,
  })
  return {
    clone_remote_url: `${publicApi}${gitPath}`,
    files,
  } satisfies RepoContent
}

export async function loadRepoLiveStateForRequest(data: RepoParams) {
  const repo = await createApiClient().get(
    repoRoute(ApiRouteTemplates.repo, data),
    RepoSummaryResponseValidator,
    { auth: 'optional' },
  )
  return repoLiveState(data, repo)
}

export async function loadRepoFileForRequest(
  data: RepoParams & { path: string; view: ViewId } & RepositoryVersionSearch,
  signal?: AbortSignal,
) {
  const api = createApiClient()
  const query = repoVersionQuery(data)
  query.set('path', data.path)
  return api.get(
    `${repoRoute(ApiRouteTemplates.repoFileContent, data)}?${query}`,
    RepoFileContentResponseValidator,
    { auth: 'optional', signal },
  )
}

export async function loadRepoDependenciesForRequest(
  data: RepoParams,
  signal?: AbortSignal,
): Promise<RepositoryDependencyCheckResponse> {
  return createApiClient().get(
    repoRoute(ApiRouteTemplates.repoDependencies, data),
    RepositoryDependencyCheckResponseValidator,
    { auth: 'required', maxResponseBytes: 8 * 1024 * 1024, signal },
  )
}

function repoVersionQuery({ request, update, view }: { view: ViewId } & RepositoryVersionSearch) {
  const query = new URLSearchParams({ view })
  if (request) query.set('request', request)
  if (update) query.set('update', update)
  return query
}

function repoLiveState(data: RepoParams, repo: RepoSummaryResponse): RepoLiveState {
  const publicApi = stripTrailingSlash(getPublicApiConnection('building repo event stream URL'))
  return {
    api_url: publicApi,
    clerk_token_template: clerkApiTokenTemplate(),
    event_stream_url: `${publicApi}${repoRoute(ApiRouteTemplates.repoEvents, data)}`,
    repo,
  }
}
