import type { RequestParams } from '@/api/types'
import type { RequestRevisionListResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import type { LoadDiscussionsInput } from './request-discussion-api'
import { requestChangeSelection, requestRevisionCommitId } from './request-changes-model'
import type { RequestDiscussionPage } from './request-discussion-types'

// Only the selected commit is loaded. One page stays within the whole-load
// ceilings of four requests, 400 references, 512 KiB, two seconds and two workers.
const DISCUSSION_REFERENCE_LIMITS = { items: 100, bytes: 512 * 1024, milliseconds: 2_000 }

type LoadReferencePage = (
  input: LoadDiscussionsInput,
  options: { signal: AbortSignal; maxResponseBytes: number },
) => Promise<RequestDiscussionPage>

type RequestRevision = RequestRevisionListResponse['revisions'][number]

export const requestDiscussionReferenceResource = createCachedResource<RequestDiscussionPage>({
  maxEntries: 16,
  maxWeight: 8 * DISCUSSION_REFERENCE_LIMITS.bytes,
  weightOf: (page) => JSON.stringify(page).length * 2,
})

export function requestDiscussionReferenceIdentity(accessScope: string, commitKey: string) {
  return `${accessScope}\0${commitKey}`
}

export function discussionReferenceQuery(
  params: RequestParams,
  revision: RequestRevision,
  commit: string,
) {
  return {
    key: requestRevisionCommitId(revision.id, commit),
    input: {
      ...params,
      commit_oid: commit,
      include_revision_anchor: commit === revision.commits.at(-1)?.oid,
      revision_id: revision.id,
    },
  }
}

export function selectedDiscussionReferenceQuery(
  params: RequestParams & { commit_oid?: string; revision_id?: string },
  revisions: RequestRevisionListResponse,
) {
  const { revision, commit } = requestChangeSelection(
    revisions.revisions,
    revisions.review_revision_id,
    { commit: params.commit_oid, revision: params.revision_id },
  )
  if (!revision || !commit) return null
  return discussionReferenceQuery(params, revision, commit)
}

// The route loader supplies the first page. A newer loader snapshot replaces
// whatever an earlier visit accumulated; an older one keeps the loaded pages.
export function openRequestDiscussionReferences(identity: string, page: RequestDiscussionPage) {
  const cached = requestDiscussionReferenceResource.peek(identity)
  if (cached && cached.snapshot_version >= page.snapshot_version) return cached
  requestDiscussionReferenceResource.write(identity, page)
  return page
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
    throw new Error('Discussions changed. Reload Changes to continue.')
  }
  if (page.next_cursor && page.next_cursor === previous.next_cursor) {
    throw new Error('Discussion reference pagination repeated a cursor.')
  }
  return { ...page, discussions: [...previous.discussions, ...page.discussions] }
}
