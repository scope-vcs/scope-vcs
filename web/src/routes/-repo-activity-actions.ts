import { loadHistoryPageForRequest } from '@/api/history'
import { parseRepoParams } from '@/api/repo-params'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

export const loadRepositoryLatestActivity = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    const page = await loadHistoryPageForRequest({ ...data, audience: null, before: null, feed: 'all' }, getRequest().signal)
    return {
      audience: page.audience,
      entry: page.entries[0] ?? null,
      head_oid: page.head_oid,
    }
  })
