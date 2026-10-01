import { createApiClient } from '@/api/client'
import type { ConnectRepoGitHubInput, GitHubSetupInput, RepoParams } from './types'
import type {
  GitHubConnectionResponse,
  GitHubInstallResponse,
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

export async function startRepoGitHubInstallForRequest(
  data: RepoParams,
): Promise<GitHubInstallResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoGitHubInstall, data),
    apiValidators.GitHubInstallResponse,
    { auth: 'required' },
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
      body: { state: data.state, installation_id: data.installation_id, code: data.code },
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
