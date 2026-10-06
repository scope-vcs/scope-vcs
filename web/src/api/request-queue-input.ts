import type { RepoParams } from './types'
import { parseRepoParams } from './repo-params'
import type { RequestQueueSection, ViewId } from './types.generated'
import { parseViewId } from './repo-views'

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
  view?: ViewId | null
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

  const view = typeof data?.view === 'string' && data.view ? parseViewId(data.view) : null

  return {
    ...params,
    cursor: cursor || null,
    search: search || null,
    section,
    view,
  }
}
