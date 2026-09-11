import type { RepoParams } from './types'
import { parseRepoParams } from './repo-params'
import type { RequestQueueSection } from './types.generated'

export type { RequestQueueSection } from './types.generated'

const REQUEST_QUEUE_SECTIONS = [
  'active',
  'unclaimed',
  'set_aside',
  'done',
] as const satisfies readonly RequestQueueSection[]

export type LoadRequestQueueInput = RepoParams & {
  cursor?: string | null
  search?: string | null
  section: RequestQueueSection
}

export function parseLoadRequestQueueInput(
  input: unknown,
): LoadRequestQueueInput {
  const data = input as Partial<LoadRequestQueueInput> | null
  const params = parseRepoParams(data)
  const cursor = typeof data?.cursor === 'string' ? data.cursor.trim() : ''
  const search = typeof data?.search === 'string' ? data.search.trim() : ''
  const section = REQUEST_QUEUE_SECTIONS.find(
    (candidate) => candidate === data?.section,
  )

  if (!section) {
    throw new Error('Request queue section is invalid.')
  }

  return {
    ...params,
    cursor: cursor || null,
    search: search || null,
    section,
  }
}
