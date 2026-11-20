import type { RequestChildrenPageResponse, RequestStateResponse } from '../../../api/types.generated'
import { createRepoResource, repoResourceKey } from '../../repo-detail/repo-resource'
import { resourceErrorMessage } from '../../../lib/use-cached-resource'

export type RequestStateValue = { state: RequestStateResponse | null; childrenPages?: number; childrenError?: string | null }

export const requestStateResource = createRepoResource<RequestStateValue>({
  invalidatedBy: ['request', 'requestTimeline', 'checks'],
  affectedBy: ({ requestId }, key, { snapshot }) => {
    if (requestId === undefined || key[0] === requestId || snapshot.value?.state?.detail.author_record) return true
    const navigation = snapshot.value?.state?.detail.stack_navigation
    return snapshot.pending || Boolean(navigation && (
      navigation.ancestors.some((node) => node.id === requestId) ||
      navigation.children.requests.some((node) => node.id === requestId)))
  },
  maxEntries: 16,
  maxWeight: 2 * 1024 * 1024,
  weightOf: ({ state }) => JSON.stringify(state).length * 2,
  coalesceInvalidations: true,
})

export function requestStateIdentity(scope: string, requestId: string) {
  return repoResourceKey(scope, requestId)
}

export function requestStateAvailable(identity: string) {
  return Boolean(requestStateResource.peek(identity)?.state)
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
  let value: RequestStateValue = { ...await load(signal), childrenPages: 1, childrenError: null }
  for (let page = 1; page < pages && loadChildren; page++) {
    const cursor = value.state?.detail.stack_navigation.children.next_cursor
    if (!cursor) break
    try {
      value = appendChildren(value, await loadChildren(cursor, signal))
    } catch (error) {
      if (signal.aborted) throw error
      return { ...value, childrenError: resourceErrorMessage(error, 'Children are unavailable.') }
    }
  }
  return value
}

export async function loadMoreRequestChildren(identity: string, load: (cursor: string, signal: AbortSignal) => Promise<RequestChildrenPageResponse>) {
  const snapshot = requestStateResource.getSnapshot(identity)
  const value = snapshot.value
  const cursor = value?.state?.detail.stack_navigation.children.next_cursor
  if (!value || !cursor || snapshot.stale || snapshot.pending) return
  try {
    const children = await load(cursor, new AbortController().signal)
    requestStateResource.writeIfUnchanged(identity, snapshot, appendChildren(value, children))
  } catch (error) {
    requestStateResource.writeIfUnchanged(identity, snapshot, { ...value, childrenError: resourceErrorMessage(error, 'Children are unavailable.') })
  }
}

function appendChildren(value: RequestStateValue, page: RequestChildrenPageResponse): RequestStateValue {
  if (!value.state) return value
  const { detail } = value.state
  const navigation = detail.stack_navigation
  const requests = new Map(navigation.children.requests.map((node) => [node.id, node]))
  for (const node of page.requests) requests.set(node.id, node)
  return {
    childrenPages: (value.childrenPages ?? 1) + 1,
    childrenError: null,
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
