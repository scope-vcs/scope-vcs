import { createApiClient } from '@/api/client'
import { parseRepoParams } from '@/api/repo-params'
import type { RepoRunHistoryInput } from '@/api/types'
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
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import {
  loadRepoRunDetailForRequest,
  loadRepoRunHistoryForRequest,
  loadRepoRunWorkflowsForRequest,
  parseRunActionInput,
  parseRepoRunHistoryInput,
} from '@/api/runs'
import { createServerFn } from '@tanstack/react-start'

export const loadRepoRunPage = createServerFn({ method: 'GET' })
  .validator((data: RepoRunHistoryInput & { githubRuns: boolean }) => ({ ...parseRepoRunHistoryInput(data), githubRuns: data.githubRuns }))
  .handler(({ data }) => loadOptionalResource(async () => {
    const api = createApiClient()
    if (data.githubRuns) {
      const [{ github }, names] = await Promise.all([
        loadRepoGitHubWorkflowRunsForRequest({ owner: data.owner, repo: data.repo }, api),
        loadRepoGitHubWorkflowNamesForRequest(data, api),
      ])
      if (!github) throw new Error('This repository no longer runs its checks on GitHub.')
      return { kind: 'github' as const, github, names }
    }
    const [history, catalog] = await Promise.all([
      loadRepoRunHistoryForRequest(data, api),
      loadRepoRunWorkflowsForRequest(data, api)
        .then((workflows) => ({ workflows, error: null }))
        .catch((error: unknown) => ({
          workflows: { workflows: [], native_runs_available: true },
          error: resourceErrorMessage(error, 'Workflow catalog unavailable.'),
        })),
    ])
    return {
      kind: 'native' as const,
      history,
      workflows: catalog.workflows,
      workflowsError: catalog.error,
    }
  }))

export const loadRepoGitHubWorkflowNames = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoGitHubWorkflowNamesForRequest(data))

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
