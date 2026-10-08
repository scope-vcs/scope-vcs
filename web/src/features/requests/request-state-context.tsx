import type { RequestParams } from '@/api/types'
import type { RequestStateResponse } from '@/api/types.generated'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { useCachedResource, useRetryOnReconnect } from '@/lib/use-cached-resource'
import { loadRequestState } from '@/routes/-request-state-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { createContext, use, useCallback, useMemo, type ReactNode } from 'react'
import { requestStateIdentity, requestStateResource, type RequestRouteState } from './request-state-resource'

type RequestStateContextValue = {
  error: string | null
  identity: string
  retry: () => void
  scope: string
  state: RequestStateResponse | null
  unavailable: boolean
  viewerId: string | null
}

const RequestStateContext = createContext<RequestStateContextValue | null>(null)

export function RequestStateProvider({ children, initial, params }: {
  children: ReactNode
  initial: RequestRouteState
  params: RequestParams
}) {
  const live = useRepoLayout()
  const { isLoaded, userId } = useAuth()
  const viewerId = isLoaded ? userId ?? null : initial.viewerId
  const scope = repoResourceScope(live.repo, viewerId)
  const identity = requestStateIdentity(scope, params.request_id)
  const initialValue = useMemo(() => scope === initial.scope ? { state: initial.state } : null, [initial.scope, initial.state, scope])
  const load = useCallback(async (signal: AbortSignal) => {
    const loaded = await loadRequestState({ data: params, signal })
    if (loaded.viewerId !== viewerId) throw new Error('The account changed while loading this request.')
    return { state: loaded.state }
  }, [params, viewerId])
  const resource = useCachedResource({
    fallbackError: 'Request state is unavailable.',
    identity,
    initialValue,
    load,
    resource: requestStateResource,
  })
  useRetryOnReconnect(resource)
  const value = resource.value ?? initialValue
  return (
    <RequestStateContext value={{
      error: resource.error,
      identity,
      retry: resource.retry,
      scope,
      state: value?.state ?? null,
      unavailable: value !== null && value.state === null,
      viewerId,
    }}>
      {children}
    </RequestStateContext>
  )
}

export function useRequestState() {
  const value = use(RequestStateContext)
  if (!value) throw new Error('Request state is unavailable outside its route.')
  return value
}
