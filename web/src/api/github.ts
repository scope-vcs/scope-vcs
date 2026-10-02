import { createApiClient, type ApiClient } from '@/api/client'
import type {
  ConnectRepoGitHubInput,
  GitHubSetupInput,
  RepoGitHubAuthorizeInput,
  RepoGitHubWorkflowRunsInput,
  RepoParams,
  SetRepoGitHubRequiredChecksInput,
  SetRepoGitHubRunImportCountInput,
} from './types'
import type {
  GitHubAuthorizeResponse,
  GitHubConnectionResponse,
  GitHubSetupResponse,
  GitHubWorkflowRunsResponse,
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
      body: {
        grant: data.grant,
        github_repository_id: data.github_repository_id,
        acknowledge_public: data.acknowledge_public,
        run_import_count: data.run_import_count,
      },
    },
  )
}

export async function setRepoGitHubRequiredChecksForRequest(
  data: SetRepoGitHubRequiredChecksInput,
): Promise<GitHubConnectionResponse> {
  return createApiClient().put(
    repoRoute(ApiRouteTemplates.repoGitHubRequiredChecks, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required', body: { names: data.names } },
  )
}

/** Allows private requests to go to a connected repository that became public. */
export async function confirmRepoGitHubPublicForRequest(
  data: RepoParams,
): Promise<GitHubConnectionResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoGitHubPublicConfirmation, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required' },
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

export async function startRepoGitHubSetupCheckForRequest(
  data: RepoParams,
): Promise<GitHubConnectionResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoGitHubSetupCheck, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required' },
  )
}

/** How many recent workflow runs the repository imports from GitHub. */
export async function setRepoGitHubRunImportCountForRequest(
  data: SetRepoGitHubRunImportCountInput,
): Promise<GitHubConnectionResponse> {
  return createApiClient().put(
    repoRoute(ApiRouteTemplates.repoGitHubRunImport, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required', body: { count: data.count } },
  )
}

/** Imports the connected repository's recent runs again with its current count. */
export async function startRepoGitHubRunImportForRequest(
  data: RepoParams,
): Promise<GitHubConnectionResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoGitHubRunImport, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required' },
  )
}

export async function loadRepoGitHubWorkflowRunsForRequest(
  data: RepoGitHubWorkflowRunsInput,
  api: ApiClient = createApiClient(),
): Promise<GitHubWorkflowRunsResponse> {
  const query = new URLSearchParams()
  if (data.workflow) query.set('workflow', data.workflow)
  if (data.after) query.set('after', data.after)
  const suffix = query.size ? `?${query}` : ''
  return api.get(
    `${repoRoute(ApiRouteTemplates.repoGitHubWorkflowRuns, data)}${suffix}`,
    apiValidators.GitHubWorkflowRunsResponse,
    { auth: 'optional' },
  )
}
