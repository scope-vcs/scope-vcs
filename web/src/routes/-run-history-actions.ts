import { createApiClient } from '@/api/client'
import { loadRepoGitHubWorkflowRunsForRequest } from '@/api/github'
import { loadOptionalResource } from '@/api/http'
import { parseRepoParams } from '@/api/repo-params'
import {
  loadRepoRunDetailForRequest,
  loadRepoRunHistoryForRequest,
  loadRepoRunWorkflowsForRequest,
  parseRunActionInput,
  parseRepoRunHistoryInput,
} from '@/api/runs'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { createServerFn } from '@tanstack/react-start'

// A repository whose checks run on GitHub lists GitHub's workflow runs; any
// other lists Scope's own.
export const loadRepoRunPage = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(async () => {
    const api = createApiClient()
    const { github } = await loadRepoGitHubWorkflowRunsForRequest(data, api)
    if (github) return { kind: 'github' as const, github }
    const [history, workflowResource] = await Promise.all([
      loadRepoRunHistoryForRequest(data, api),
      loadRepoRunWorkflowsForRequest(data, api)
        .then((workflows) => ({ error: null, workflows }))
        .catch((error: unknown) => ({
          error: resourceErrorMessage(error, 'Workflow catalog unavailable.'),
          // Unknown availability keeps the controls; enqueuing still enforces it.
          workflows: { workflows: [], native_runs_available: true },
        })),
    ])
    return {
      kind: 'native' as const,
      history,
      workflows: workflowResource.workflows,
      workflowsError: workflowResource.error,
    }
  }))

export const loadRepoRunWorkflows = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(() => loadRepoRunWorkflowsForRequest(data)))

export const loadRepoGitHubWorkflowRuns = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoGitHubWorkflowRunsForRequest(data))

export const loadRepoRunHistory = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(() => loadRepoRunHistoryForRequest(data)))

export const loadRepoRunDetail = createServerFn({ method: 'GET' })
  .validator(parseRunActionInput)
  .handler(({ data }) => loadRepoRunDetailForRequest(data))
