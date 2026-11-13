import { createApiClient } from '@/api/client'
import { parseRepoParams } from '@/api/repo-params'
import {
  loadRepoGitHubWorkflowJobLogForRequest,
  loadRepoGitHubWorkflowRunForRequest,
  loadRepoGitHubWorkflowRunsForRequest,
  loadRepoGitHubWorkflowNamesForRequest,
} from '@/api/github'
import {
  parseRepoGitHubWorkflowJobLogInput,
  parseRepoGitHubWorkflowRunInput,
  parseRepoGitHubWorkflowRunsInput,
} from '@/api/github-inputs'
import { loadOptionalResource } from '@/api/http'
import type { RunsPage } from '@/features/runs/runs-page-resource'
import { createServerFn } from '@tanstack/react-start'

export const loadRepoRunPage = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }): Promise<RunsPage> => {
    const api = createApiClient()
    const loaded = await loadOptionalResource(() => Promise.all([
      loadRepoGitHubWorkflowRunsForRequest(data, api),
      loadRepoGitHubWorkflowNamesForRequest(data, api),
    ]))
    if (!loaded) return { kind: 'denied' }
    const [{ configured, github }, names] = loaded
    return github ? { kind: 'github', github, names } : { kind: 'not-connected', configured }
  })

export const loadRepoGitHubWorkflowNames = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoGitHubWorkflowNamesForRequest(data))

export const loadRepoGitHubWorkflowRuns = createServerFn({ method: 'GET' })
  .validator(parseRepoGitHubWorkflowRunsInput)
  .handler(({ data }) => loadRepoGitHubWorkflowRunsForRequest(data))

export const loadRepoGitHubWorkflowRun = createServerFn({ method: 'GET' })
  .validator(parseRepoGitHubWorkflowRunInput)
  .handler(({ data }) => loadRepoGitHubWorkflowRunForRequest(data))

export const loadRepoGitHubWorkflowJobLog = createServerFn({ method: 'GET' })
  .validator(parseRepoGitHubWorkflowJobLogInput)
  .handler(({ data }) => loadRepoGitHubWorkflowJobLogForRequest(data))
