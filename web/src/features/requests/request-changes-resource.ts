import type { RequestRevisions } from '../../api/types'
import { createCachedResource } from '../../lib/cached-resource'
import { appendDiscussionReferencePage } from './request-changes-discussion-references'
import type { RequestDiscussionPage } from './request-discussion-types'

const limits = { maxEntries: 48, maxWeight: 8 * 1024 * 1024, weightOf: (value: object) => JSON.stringify(value).length * 2 }
export const requestChangesResource = createCachedResource<RequestRevisions>(limits)
export const requestDiscussionReferenceResource = createCachedResource<RequestDiscussionPage>(limits)

export function requestChangesIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}

export function requestChangesSelectionIdentity(scope: string, requestId: string, revision?: string, commit?: string) {
  return `${requestChangesIdentity(scope, requestId)}\0${revision ?? ''}\0${commit ?? ''}`
}

export function requestDiscussionReferenceIdentity(scope: string, requestId: string, commitKey: string) {
  return `${requestChangesIdentity(scope, requestId)}\0${commitKey}`
}

export function loadMoreDiscussionReferences(
  identity: string,
  load: (cursor: string, signal: AbortSignal) => Promise<RequestDiscussionPage>,
) {
  const snapshot = requestDiscussionReferenceResource.getSnapshot(identity)
  const previous = snapshot.value
  if (!previous?.next_cursor || snapshot.pending || snapshot.stale || snapshot.error) return
  const cursor = previous.next_cursor
  requestDiscussionReferenceResource.invalidate(identity)
  return requestDiscussionReferenceResource.ensure(identity, '', async (signal) =>
    appendDiscussionReferencePage(previous, await load(cursor, signal)))
}
