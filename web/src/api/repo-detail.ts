import {
  createApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { arrayOf, loadOptionalResource, stripTrailingSlash } from './http'
import { repoRoute } from './paths'
import type { RepoContent, RepoLiveState, RepoParams } from './types'
import type { RepoSummaryResponse, RepositoryDependencyCheckResponse, ViewId } from './types.generated'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  GitHubConnectionResponseValidator,
  RepoFileContentResponseValidator,
  RepoFileResponseValidator,
  RepoSummaryResponseValidator,
  RepositoryDependencyCheckResponseValidator,
} from './validators.generated'

export async function loadRepoContentForRequest(
  data: RepoParams & { view: ViewId },
  signal?: AbortSignal,
) {
  const api = createApiClient()
  const files = await api.get(
    `${repoRoute(ApiRouteTemplates.repoFiles, data)}?${new URLSearchParams({ view: data.view })}`,
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
  const api = createApiClient()
  const [repo, github] = await Promise.all([
    api.get(
      repoRoute(ApiRouteTemplates.repo, data),
      RepoSummaryResponseValidator,
      { auth: 'optional' },
    ),
    loadOptionalResource(() => api.get(
      repoRoute(ApiRouteTemplates.repoGitHub, data),
      GitHubConnectionResponseValidator,
      { auth: 'optional' },
    )),
  ])
  return repoLiveState(data, repo, Boolean(github?.configured && github.connection), github?.configured ?? false)
}

export async function loadRepoFileForRequest(
  data: RepoParams & { path: string; view: ViewId },
  signal?: AbortSignal,
) {
  const api = createApiClient()
  return api.get(
    `${repoRoute(ApiRouteTemplates.repoFileContent, data)}?${new URLSearchParams({ path: data.path, view: data.view })}`,
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

function repoLiveState(data: RepoParams, repo: RepoSummaryResponse, githubRuns: boolean, githubConfigured: boolean): RepoLiveState {
  const publicApi = stripTrailingSlash(getPublicApiConnection('building repo event stream URL'))
  return {
    api_url: publicApi,
    clerk_token_template: clerkApiTokenTemplate(),
    event_stream_url: `${publicApi}${repoRoute(ApiRouteTemplates.repoEvents, data)}`,
    repo,
    githubRuns,
    githubConfigured,
  }
}
