import {
  createApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { arrayOf, stripTrailingSlash } from './http'
import { repoRoute } from './paths'
import type { RepoContent, RepoLiveState, RepoParams } from './types'
import type { RepoSummaryResponse } from './types.generated'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'

export async function loadRepoContentForRequest(data: RepoParams, signal?: AbortSignal) {
  const api = createApiClient()
  const files = await api.get(
    repoRoute(ApiRouteTemplates.repoFiles, data),
    arrayOf(apiValidators.RepoFileResponse),
    { auth: 'optional', signal },
  )

  const publicApi = stripTrailingSlash(getPublicApiConnection('building clone command'))
  const gitPath = buildApiPath(ApiRouteTemplates.gitRepo, {
    mode: 'public',
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
  const repo = await api.get(
    repoRoute(ApiRouteTemplates.repo, data),
    apiValidators.RepoSummaryResponse,
    { auth: 'optional' },
  )
  return repoLiveState(data, repo)
}

export async function loadRepoFileForRequest(
  data: RepoParams & { path: string },
  signal?: AbortSignal,
) {
  const api = createApiClient()
  return api.get(
    `${repoRoute(ApiRouteTemplates.repoFileContent, data)}?path=${encodeURIComponent(data.path)}`,
    apiValidators.RepoFileContentResponse,
    { auth: 'optional', signal },
  )
}

function repoLiveState(data: RepoParams, repo: RepoSummaryResponse): RepoLiveState {
  const publicApi = stripTrailingSlash(getPublicApiConnection('building repo event stream URL'))
  return {
    clerk_token_template: clerkApiTokenTemplate(),
    event_stream_url: `${publicApi}${repoRoute(ApiRouteTemplates.repoEvents, data)}`,
    repo,
  }
}
