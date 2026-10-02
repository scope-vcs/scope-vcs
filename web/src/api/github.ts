import { createApiClient } from '@/api/client'
import type {
  ConnectRepoGitHubInput,
  GitHubSetupInput,
  RepoGitHubAuthorizeInput,
  RepoParams,
} from './types'
import type {
  GitHubAuthorizeResponse,
  GitHubConnectionResponse,
  GitHubSetupResponse,
} from './types.generated'
import { repoRoute } from './paths'
import { ApiRouteTemplates } from './types.generated'
import { apiValidators } from './validators.generated'

export async function loadRepoGitHubConnectionForRequest(
  data: RepoParams,
  signal?: AbortSignal,
): Promise<GitHubConnectionResponse> {
  return createApiClient().get(
    repoRoute(ApiRouteTemplates.repoGitHub, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required', signal },
  )
}

/**
 * The origin of the page connecting GitHub, so GitHub can return to the same
 * address, such as a development stack reached over a tailnet. The API
 * accepts it only when it is an allowed Scope web origin.
 */
export function currentWebOrigin() {
  return typeof window === 'undefined' ? null : window.location.origin
}

export async function startRepoGitHubAuthorizationForRequest(
  data: RepoGitHubAuthorizeInput,
): Promise<GitHubAuthorizeResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoGitHubAuthorize, data),
    apiValidators.GitHubAuthorizeResponse,
    { auth: 'required', body: { web_origin: data.web_origin } },
  )
}

export async function completeGitHubSetupForRequest(
  data: GitHubSetupInput,
): Promise<GitHubSetupResponse> {
  return createApiClient().post(
    ApiRouteTemplates.githubSetup,
    apiValidators.GitHubSetupResponse,
    {
      auth: 'required',
      body: { state: data.state, code: data.code },
    },
  )
}

export async function connectRepoGitHubForRequest(
  data: ConnectRepoGitHubInput,
): Promise<GitHubConnectionResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoGitHub, data),
    apiValidators.GitHubConnectionResponse,
    {
      auth: 'required',
      body: { grant: data.grant, github_repository_id: data.github_repository_id },
    },
  )
}

export async function disconnectRepoGitHubForRequest(
  data: RepoParams,
): Promise<GitHubConnectionResponse> {
  return createApiClient().delete(
    repoRoute(ApiRouteTemplates.repoGitHub, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required' },
  )
}
