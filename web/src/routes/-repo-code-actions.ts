import { parseRepoFileInput } from '@/api/request-inputs'
import { HttpError, isNotFoundError, loadOptionalResource } from '@/api/http'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  loadRepoLanguagesForRequest,
} from '@/api/repo-detail'
import { parseRepoVersionViewInput, parseRepoViewInput } from '@/api/repo-params'
import type { RepoFileLoadResult } from '@/features/repo-detail/repo-code-route-data'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

const PROJECTION_REBUILDING_MESSAGE = 'repository projection is rebuilding; retry shortly'

export const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoVersionViewInput)
  .handler(({ data }) => loadRepoContentForRequest(data, getRequest().signal))

export const loadRepoLanguages = createServerFn({ method: 'GET' })
  .validator(parseRepoViewInput)
  .handler(async ({ data }) => {
    const signal = getRequest().signal
    return (await loadOptionalResource(() => loadRepoLanguagesForRequest(data, signal))) ?? []
  })

export const loadRepoFile = createServerFn({ method: 'GET' })
  .validator(parseRepoFileInput)
  .handler(async ({ data }): Promise<RepoFileLoadResult> => {
    try {
      return { file: await loadRepoFileForRequest(data, getRequest().signal), status: 'ready' }
    } catch (error) {
      if (isNotFoundError(error)) return { status: 'missing' }
      if (
        error instanceof HttpError &&
        error.status === 503 &&
        error.message === PROJECTION_REBUILDING_MESSAGE
      ) {
        return { status: 'rebuilding' }
      }
      throw error
    }
  })
