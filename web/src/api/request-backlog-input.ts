import type { RepoParams } from './types'
import { parseRepoParams } from './repo-params'
import type { RequestBacklogSort, RequestBacklogWorking, ViewId } from './types.generated'
import { parseViewId } from './repo-views'

export const REQUEST_BACKLOG_WORKING = ['anyone', 'nobody', 'me'] as const satisfies readonly RequestBacklogWorking[]
export const REQUEST_BACKLOG_SORTS = ['oldest', 'newest'] as const satisfies readonly RequestBacklogSort[]

export type RequestBacklogQuery = {
  search: string
  working: RequestBacklogWorking
  sort: RequestBacklogSort
}

export type LoadRequestBacklogInput = RepoParams & RequestBacklogQuery & {
  cursor: string | null
  limit: number | null
  view: ViewId | null
}

export function parseRequestBacklogQuery(input: { search?: unknown; working?: unknown; sort?: unknown }): RequestBacklogQuery {
  return {
    search: typeof input.search === 'string' ? input.search.trim() : '',
    working: REQUEST_BACKLOG_WORKING.find((working) => working === input.working) ?? 'anyone',
    sort: REQUEST_BACKLOG_SORTS.find((sort) => sort === input.sort) ?? 'oldest',
  }
}

export function parseLoadRequestBacklogInput(input: unknown): LoadRequestBacklogInput {
  const data = (input ?? {}) as Partial<Record<keyof LoadRequestBacklogInput, unknown>>
  return {
    ...parseRepoParams(data),
    ...parseRequestBacklogQuery(data),
    cursor: typeof data.cursor === 'string' && data.cursor.trim() ? data.cursor.trim() : null,
    limit: typeof data.limit === 'number' && Number.isInteger(data.limit) && data.limit > 0 ? data.limit : null,
    view: typeof data.view === 'string' && data.view ? parseViewId(data.view) : null,
  }
}
