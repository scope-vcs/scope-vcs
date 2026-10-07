import type { RunActionInput } from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useEffect } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { startRunLogStream } from './run-log-cache'

export function useRunLogStream(key: string | null, params: RunActionInput) {
  const { getToken } = useAuth()
  const { api_url, clerk_token_template } = useRepoLayout()
  const url = `${api_url}${buildApiPath(ApiRouteTemplates.repoRunEvents, params)}`

  useEffect(() => {
    if (!key) return
    return startRunLogStream(key, url, clerk_token_template, getToken)
  }, [clerk_token_template, getToken, key, url])
}
