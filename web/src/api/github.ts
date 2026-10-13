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
  GitHubWorkflowNamesResponse,
} from './types.generated'
import { repoRoute } from './paths'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  GitHubAuthorizeResponseValidator,
  GitHubConnectionResponseValidator,
  GitHubSetupResponseValidator,
  GitHubWorkflowJobLogResponseValidator,
  GitHubWorkflowNamesResponseValidator,
  GitHubWorkflowRunDetailResponseValidator,
  GitHubWorkflowRunsResponseValidator,
} from './validators.generated'

export async function loadRepoGitHubConnectionForRequest(
  data: RepoParams,
  signal?: AbortSignal,
): Promise<GitHubConnectionResponse> {
  return createApiClient().get(
    repoRoute(ApiRouteTemplates.repoGitHub, data),
    GitHubConnectionResponseValidator,
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
    GitHubAuthorizeResponseValidator,
    { auth: 'required', body: { web_origin: data.web_origin } },
  )
}

export async function completeGitHubSetupForRequest(
  data: GitHubSetupInput,
): Promise<GitHubSetupResponse> {
  return createApiClient().post(
    ApiRouteTemplates.githubSetup,
    GitHubSetupResponseValidator,
    {
      auth: 'required',
      body: { state: data.state, code: data.code },
    },
  )
}

function changeRepoGitHub(
  method: 'delete' | 'post' | 'put',
  template: string,
  data: RepoParams,
  body?: unknown,
): Promise<GitHubConnectionResponse> {
  return createApiClient()[method](
    repoRoute(template, data),
    GitHubConnectionResponseValidator,
    { auth: 'required', body },
  )
}

export async function connectRepoGitHubForRequest(data: ConnectRepoGitHubInput) {
  return changeRepoGitHub('post', ApiRouteTemplates.repoGitHub, data, {
    grant: data.grant,
    github_repository_id: data.github_repository_id,
    acknowledge_public: data.acknowledge_public,
    run_import_count: data.run_import_count,
  })
}

export async function setRepoGitHubRequiredChecksForRequest(data: SetRepoGitHubRequiredChecksInput) {
  return changeRepoGitHub('put', ApiRouteTemplates.repoGitHubRequiredChecks, data, { names: data.names })
}

export async function confirmRepoGitHubPublicForRequest(data: RepoParams) {
  return changeRepoGitHub('post', ApiRouteTemplates.repoGitHubPublicConfirmation, data)
}

export async function disconnectRepoGitHubForRequest(data: RepoParams) {
  return changeRepoGitHub('delete', ApiRouteTemplates.repoGitHub, data)
}

export async function startRepoGitHubSetupCheckForRequest(data: RepoParams) {
  return changeRepoGitHub('post', ApiRouteTemplates.repoGitHubSetupCheck, data)
}

export async function setRepoGitHubRunImportCountForRequest(data: SetRepoGitHubRunImportCountInput) {
  return changeRepoGitHub('put', ApiRouteTemplates.repoGitHubRunImport, data, { count: data.count })
}

export async function startRepoGitHubRunImportForRequest(data: RepoParams) {
  return changeRepoGitHub('post', ApiRouteTemplates.repoGitHubRunImport, data)
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
    GitHubWorkflowRunsResponseValidator,
    { auth: 'optional' },
  )
}

export async function loadRepoGitHubWorkflowNamesForRequest(
  data: RepoParams,
  api: ApiClient = createApiClient(),
): Promise<GitHubWorkflowNamesResponse> {
  return api.get(
    repoRoute(ApiRouteTemplates.repoGitHubWorkflowNames, data),
    GitHubWorkflowNamesResponseValidator,
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
    GitHubWorkflowRunDetailResponseValidator,
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
    GitHubWorkflowJobLogResponseValidator,
    { auth: 'optional' },
  )
}
