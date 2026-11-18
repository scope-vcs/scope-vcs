import type { RequestParams } from '@/api/types'
import type { RequestComparisonMode, RequestRevisionListResponse } from '@/api/types.generated'
import { affectsRequestKey, createRepoResource, repoResourceKey } from '../repo-detail/repo-resource'
import type { LoadDiscussionsInput } from './request-discussion-api'
import { latestRequestRevision } from './request-changes-model'
import type { RequestDiscussionPage } from './request-discussion-types'

const DISCUSSION_REFERENCE_LIMITS = { items: 100, bytes: 512 * 1024, milliseconds: 2_000 }

type LoadReferencePage = (
  input: LoadDiscussionsInput,
  options: { signal: AbortSignal; maxResponseBytes: number },
) => Promise<RequestDiscussionPage>

type RequestRevision = RequestRevisionListResponse['revisions'][number]

export const requestDiscussionReferenceResource = createRepoResource<RequestDiscussionPage>({
  invalidatedBy: ['requestHead', 'requestTimeline'],
  affectedBy: affectsRequestKey,
  maxEntries: 16,
  maxWeight: 8 * DISCUSSION_REFERENCE_LIMITS.bytes,
  weightOf: (page) => JSON.stringify(page).length * 2,
})

export function requestDiscussionReferenceIdentity(accessScope: string, requestId: string, referenceKey: string) {
  return repoResourceKey(accessScope, requestId, referenceKey)
}

export function comparisonDiscussionReferenceQuery(
  params: RequestParams,
  revisions: RequestRevision[],
  revision: RequestRevision,
  mode: RequestComparisonMode,
  commit: string | null,
) {
  if (mode === 'commit' && !commit) return null
  const filter = {
    commit_oid: mode === 'commit' ? commit ?? undefined : undefined,
    include_carried_line_anchors: mode !== 'commit' && revision.id === latestRequestRevision(revisions)?.id
      ? true
      : undefined,
    include_revision_anchor: mode === 'commit' && commit === revision.commits.at(-1)?.oid,
    revision_id: revision.id,
  }
  return { key: JSON.stringify(filter), input: { ...params, ...filter } }
}

export function refreshRequestDiscussionReferences(accessScope: string, requestId: string) {
  requestDiscussionReferenceResource.invalidateFor(accessScope, [{ topic: 'requestTimeline', requestId }])
}

export function loadMoreRequestDiscussionReferences(
  identity: string,
  loadPage: (cursor: string) => Promise<RequestDiscussionPage>,
) {
  const current = requestDiscussionReferenceResource.peek(identity)
  if (!current?.next_cursor) return Promise.resolve(current)
  const cursor = current.next_cursor
  requestDiscussionReferenceResource.invalidate(identity)
  return requestDiscussionReferenceResource.ensure(identity, '', async () =>
    appendDiscussionReferencePage(current, await loadPage(cursor)))
}

export async function loadDiscussionReferencePage(
  input: LoadDiscussionsInput,
  loadPage: LoadReferencePage,
): Promise<RequestDiscussionPage> {
  const controller = new AbortController()
  let timer: ReturnType<typeof setTimeout> | undefined
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      controller.abort()
      reject(new Error('Loading discussion references timed out.'))
    }, DISCUSSION_REFERENCE_LIMITS.milliseconds)
  })
  try {
    const page = await Promise.race([
      loadPage({ ...input, limit: DISCUSSION_REFERENCE_LIMITS.items }, {
        signal: controller.signal,
        maxResponseBytes: DISCUSSION_REFERENCE_LIMITS.bytes,
      }),
      deadline,
    ])
    if (page.discussions.length > DISCUSSION_REFERENCE_LIMITS.items
      || new TextEncoder().encode(JSON.stringify(page)).byteLength > DISCUSSION_REFERENCE_LIMITS.bytes) {
      throw new Error('Discussion references exceed the page limit.')
    }
    return page
  } finally {
    clearTimeout(timer)
  }
}

export function appendDiscussionReferencePage(
  previous: RequestDiscussionPage,
  page: RequestDiscussionPage,
): RequestDiscussionPage {
  if (page.snapshot_version !== previous.snapshot_version) {
    throw new Error('Discussions changed. Retry to refresh the references.')
  }
  if (page.next_cursor && page.next_cursor === previous.next_cursor) {
    throw new Error('Discussion reference pagination repeated a cursor.')
  }
  return { ...page, discussions: [...previous.discussions, ...page.discussions] }
}
