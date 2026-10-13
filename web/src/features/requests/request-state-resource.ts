import type { RequestChildrenPageResponse, RequestStateResponse } from '../../api/types.generated'
import type { RepoLiveState } from '../../api/types'
import { createCachedResource } from '../../lib/cached-resource'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'

export type RequestStateValue = { state: RequestStateResponse | null; childrenPages?: number }

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
  loadChildren?: (cursor: string, signal: AbortSignal) => Promise<RequestChildrenPageResponse>,
) {
  try {
    return await requestStateResource.load(identity, '', (signal) => reloadRequestStateValue(identity, load, loadChildren, signal))
  } catch (error) {
    const retained = requestStateResource.peek(identity)
    if (retained) return retained
    throw error
  }
}

export async function reloadRequestStateValue(identity: string, load: (signal: AbortSignal) => Promise<RequestStateValue>, loadChildren: ((cursor: string, signal: AbortSignal) => Promise<RequestChildrenPageResponse>) | undefined, signal: AbortSignal) {
  const pages = requestStateResource.peek(identity)?.childrenPages ?? 1
  let value = await load(signal)
  for (let page = 1; page < pages && loadChildren; page++) {
    const cursor = value.state?.detail.stack_navigation.children.next_cursor
    if (!cursor) break
    value = appendChildren(value, await loadChildren(cursor, signal))
  }
  return value
}

export function refreshRequestState(scope: string, requestId?: string) {
  requestStateResource.invalidateMatching((identity) => {
    if (!identity.startsWith(`${scope}\0`)) return false
    if (!requestId || identity === requestStateIdentity(scope, requestId)) return true
    const snapshot = requestStateResource.getSnapshot(identity)
    const navigation = snapshot.value?.state?.detail.stack_navigation
    return snapshot.pending || Boolean(navigation && (
      navigation.ancestors.some((node) => node.id === requestId) ||
      navigation.children.requests.some((node) => node.id === requestId)))
  })
}

export async function loadMoreRequestChildren(identity: string, load: (cursor: string, signal: AbortSignal) => Promise<RequestChildrenPageResponse>) {
  const snapshot = requestStateResource.getSnapshot(identity)
  const value = snapshot.value
  const cursor = value?.state?.detail.stack_navigation.children.next_cursor
  if (!value || !cursor || snapshot.stale || snapshot.pending) return
  const children = await load(cursor, new AbortController().signal)
  requestStateResource.writeIfUnchanged(identity, snapshot, appendChildren(value, children))
}

function appendChildren(value: RequestStateValue, page: RequestChildrenPageResponse): RequestStateValue {
  if (!value.state) return value
  const { detail } = value.state
  const navigation = detail.stack_navigation
  const requests = new Map(navigation.children.requests.map((node) => [node.id, node]))
  for (const node of page.requests) requests.set(node.id, node)
  return {
    childrenPages: (value.childrenPages ?? 1) + 1,
    state: { ...value.state, detail: { ...detail, stack_navigation: { ...navigation, children: { requests: [...requests.values()], next_cursor: page.next_cursor } } } },
  }
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
  return requestStateResource.writeIfUnchanged(identity, snapshot, { ...snapshot.value, state: next })
}
