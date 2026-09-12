import { createApiClient } from '@/api/client'
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
    const [history, workflowResource] = await Promise.all([
      loadRepoRunHistoryForRequest(data, api),
      loadRepoRunWorkflowsForRequest(data, api)
        .then((workflows) => ({ error: null, workflows }))
        .catch((error: unknown) => ({
          error: resourceErrorMessage(error, 'Workflow catalog unavailable.'),
          workflows: { workflows: [] },
        })),
    ])
    return {
      history,
      workflows: workflowResource.workflows,
      workflowsError: workflowResource.error,
    }
  }))

export const loadRepoRunHistory = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(({ data }) => loadOptionalResource(() => loadRepoRunHistoryForRequest(data)))

export const loadRepoRunDetail = createServerFn({ method: 'GET' })
  .validator(parseRunActionInput)
  .handler(({ data }) => loadRepoRunDetailForRequest(data))
