import type { RequestStateResponse } from '../../api/types.generated'
import type { RepoLiveState } from '../../api/types'
import { createCachedResource } from '../../lib/cached-resource'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'

export type RequestStateValue = { state: RequestStateResponse | null }

export type RequestRouteState = RequestStateValue & {
  scope: string
  viewerId: string | null
}

export function requestRouteState(live: RepoLiveState, value: RequestStateValue, viewerId: string | null): RequestRouteState {
  return { ...value, scope: repoResourceScope(live.repo, viewerId), viewerId }
}

export const requestStateResource = createCachedResource<RequestStateValue>({
  maxEntries: 16,
  maxWeight: 2 * 1024 * 1024,
  weightOf: ({ state }) => JSON.stringify(state).length * 2,
  coalesceInvalidations: true,
})

export function requestStateIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}

export async function loadRequestStateValue(
  identity: string,
  load: (signal: AbortSignal) => Promise<RequestStateValue>,
) {
  try {
    return await requestStateResource.load(identity, '', load)
  } catch (error) {
    const retained = requestStateResource.peek(identity)
    if (retained) return retained
    throw error
  }
}

export function refreshRequestState(scope: string, requestId?: string) {
  if (requestId) requestStateResource.invalidate(requestStateIdentity(scope, requestId))
  else requestStateResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
}

export function reconcileRequestState(
  identity: string,
  snapshot: ReturnType<typeof requestStateResource.getSnapshot>,
  update: (state: RequestStateResponse) => RequestStateResponse,
) {
  const state = snapshot.value?.state
  if (!state) return false
  const next = update(state)
  const requestId = state.detail.request.id
  const head = state.detail.request.head_oid
  if (next.detail.request.id !== requestId || next.checks.request_id !== requestId || next.auto_merge.request_id !== requestId) return false
  if (next.detail.request.head_oid !== head || next.checks.head_oid !== head || next.auto_merge.head_oid !== head) return false
  return requestStateResource.writeIfUnchanged(identity, snapshot, { state: next })
}
