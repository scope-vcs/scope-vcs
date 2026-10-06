import { loadHistoryPageForRequest } from '@/api/history'
import { parseRepoViewInput } from '@/api/repo-params'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

export const loadRepositoryLatestActivity = createServerFn({ method: 'GET' })
  .validator(parseRepoViewInput)
  .handler(async ({ data }) => {
    const page = await loadHistoryPageForRequest({ ...data, before: null, feed: 'all' }, getRequest().signal)
    return {
      view: page.view,
      entry: page.entries[0] ?? null,
      head_oid: page.head_oid,
    }
  })
