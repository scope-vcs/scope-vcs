import { createApiClient } from '@/api/client'
import {
  loadRepoGitHubWorkflowJobLogForRequest,
  loadRepoGitHubWorkflowRunForRequest,
  loadRepoGitHubWorkflowRunsForRequest,
} from '@/api/github'
import {
  parseRepoGitHubWorkflowJobLogInput,
  parseRepoGitHubWorkflowRunInput,
  parseRepoGitHubWorkflowRunsInput,
} from '@/api/github-inputs'
import { loadOptionalResource } from '@/api/http'
import {
  loadRepoRunDetailForRequest,
  loadRepoRunHistoryForRequest,
  loadRepoRunWorkflowsForRequest,
  parseRunActionInput,
  parseRepoRunHistoryInput,
} from '@/api/runs'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { createServerFn } from '@tanstack/react-start'

export const loadRepoRunPage = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(async () => {
    const api = createApiClient()
    const { configured, github } = await loadRepoGitHubWorkflowRunsForRequest(
      { owner: data.owner, repo: data.repo },
      api,
    )
    if (github) return { kind: 'github' as const, github }
    const [history, workflowResource] = await Promise.all([
      loadRepoRunHistoryForRequest(data, api),
      loadRepoRunWorkflowsForRequest(data, api)
        .then((workflows) => ({ error: null, workflows }))
        .catch((error: unknown) => ({
          error: resourceErrorMessage(error, 'Workflow catalog unavailable.'),
          workflows: { workflows: [], native_runs_available: true },
        })),
    ])
    return {
      kind: 'native' as const,
      githubConfigured: configured,
      history,
      workflows: workflowResource.workflows,
      workflowsError: workflowResource.error,
    }
  }))

export const loadRepoRunWorkflows = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(() => loadRepoRunWorkflowsForRequest(data)))

export const loadRepoGitHubWorkflowRuns = createServerFn({ method: 'GET' })
  .validator(parseRepoGitHubWorkflowRunsInput)
  .handler(({ data }) => loadRepoGitHubWorkflowRunsForRequest(data))

export const loadRepoGitHubWorkflowRun = createServerFn({ method: 'GET' })
  .validator(parseRepoGitHubWorkflowRunInput)
  .handler(({ data }) => loadRepoGitHubWorkflowRunForRequest(data))

export const loadRepoGitHubWorkflowJobLog = createServerFn({ method: 'GET' })
  .validator(parseRepoGitHubWorkflowJobLogInput)
  .handler(({ data }) => loadRepoGitHubWorkflowJobLogForRequest(data))

export const loadRepoRunHistory = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(() => loadRepoRunHistoryForRequest(data)))

export const loadRepoRunDetail = createServerFn({ method: 'GET' })
  .validator(parseRunActionInput)
  .handler(({ data }) => loadRepoRunDetailForRequest(data))
