import { currentWebOrigin, loadRepoGitHubConnectionForRequest, startRepoGitHubAuthorizationForRequest } from '@/api/github'
import { parseRepoGitHubAuthorizeInput } from '@/api/github-inputs'
import { loadOptionalResource } from '@/api/http'
import { parseRepoParams } from '@/api/repo-params'
import { loadRepoCollaborationForRequest } from '@/api/repo-settings'
import type { RepoParams } from '@/api/types'
import type { RepoSettingsData } from '@/features/repo-detail/repo-settings-resource'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

const loadRepoCollaboration = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadOptionalResource(() => loadRepoCollaborationForRequest(data, getRequest().signal)))

const loadRepoGitHubConnection = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadOptionalResource(() => loadRepoGitHubConnectionForRequest(data, getRequest().signal)))

const startRepoGitHubAuthorizationFn = createServerFn({ method: 'POST' })
  .validator(parseRepoGitHubAuthorizeInput)
  .handler(({ data }) => startRepoGitHubAuthorizationForRequest(data))

/**
 * The settings page's server data. The Runs page reads the GitHub connection
 * from the same cached resource, so both show the same state.
 */
export async function loadRepoSettingsData(params: RepoParams, signal: AbortSignal): Promise<RepoSettingsData> {
  const data = { owner: params.owner, repo: params.repo }
  const [collaboration, github] = await Promise.all([
    loadRepoCollaboration({ data, signal }),
    loadRepoGitHubConnection({ data, signal }),
  ])
  return { collaboration, github }
}

/** GitHub's authorization screen for connecting the repository from this origin. */
export function startRepoGitHubAuthorization(params: RepoParams) {
  return startRepoGitHubAuthorizationFn({
    data: { owner: params.owner, repo: params.repo, web_origin: currentWebOrigin() },
  })
}
