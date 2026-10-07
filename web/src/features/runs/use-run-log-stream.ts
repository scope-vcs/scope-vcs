import type { RunActionInput } from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useEffect } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { startRunLogStream } from './run-log-cache'

export function useRunLogStream(key: string | null, params: RunActionInput) {
  const { getToken } = useAuth()
  const { event_stream_url, clerk_token_template } = useRepoLayout()
  const repoEventsPath = buildApiPath(ApiRouteTemplates.repoEvents, params)
  const runEventsPath = buildApiPath(ApiRouteTemplates.repoRunEvents, params)
  const url = `${event_stream_url.slice(0, -repoEventsPath.length)}${runEventsPath}`

  useEffect(() => {
    if (!key) return
    return startRunLogStream(key, url, clerk_token_template, getToken)
  }, [clerk_token_template, getToken, key, url])
}
