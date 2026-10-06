import { createApiClient, type ApiClient } from '@/api/client'
import type {
  ConnectRepoGitHubInput,
  GitHubSetupInput,
  RepoGitHubAuthorizeInput,
  RepoGitHubWorkflowJobLogInput,
  RepoGitHubWorkflowRunsInput,
  RepoParams,
  RunActionInput,
  SetRepoGitHubRequiredChecksInput,
  SetRepoGitHubRunImportCountInput,
} from './types'
import type {
  GitHubAuthorizeResponse,
  GitHubConnectionResponse,
  GitHubSetupResponse,
  GitHubWorkflowJobLogResponse,
  GitHubWorkflowRunDetailResponse,
  GitHubWorkflowRunsResponse,
} from './types.generated'
import { repoRoute } from './paths'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
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

export async function setRepoGitHubRunImportCountForRequest(
  data: SetRepoGitHubRunImportCountInput,
): Promise<GitHubConnectionResponse> {
  return createApiClient().put(
    repoRoute(ApiRouteTemplates.repoGitHubRunImport, data),
    apiValidators.GitHubConnectionResponse,
    { auth: 'required', body: { count: data.count } },
  )
}

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

export async function loadRepoGitHubWorkflowRunForRequest(
  data: RunActionInput,
): Promise<GitHubWorkflowRunDetailResponse> {
  return createApiClient().get(
    buildApiPath(ApiRouteTemplates.repoGitHubWorkflowRun, {
      owner: data.owner,
      repo: data.repo,
      run_id: data.run_id,
    }),
    apiValidators.GitHubWorkflowRunDetailResponse,
    { auth: 'optional' },
  )
}

export async function loadRepoGitHubWorkflowJobLogForRequest(
  data: RepoGitHubWorkflowJobLogInput,
): Promise<GitHubWorkflowJobLogResponse> {
  return createApiClient().get(
    buildApiPath(ApiRouteTemplates.repoGitHubWorkflowJobLog, {
      owner: data.owner,
      repo: data.repo,
      run_id: data.run_id,
      job_id: data.job_id,
    }),
    apiValidators.GitHubWorkflowJobLogResponse,
    { auth: 'optional' },
  )
}
