import { useState } from 'react'
import type { RequestRevisions } from '@/api/types'
import { useCachedResource } from '@/lib/use-cached-resource'
import { requestChangesResource } from './request-changes-resource'
import type { RequestChangesDiscussionReferences } from './request-changes-workbench'

type InitialChanges = {
  revisions: RequestRevisions | null
  discussionReferences: RequestChangesDiscussionReferences
}

// Loader data is a hydration seed for its original viewer and access scope.
// The resource owns subsequent reads, refreshes and retained navigation data.
export function useRequestChangesResource({
  access, identity, initial, initialViewerId, load, viewerId,
}: {
  access: string
  identity: string | null
  initial: InitialChanges | null
  initialViewerId: string | null
  load: (signal: AbortSignal) => Promise<RequestRevisions>
  viewerId: string | null
}) {
  const [initialAccess] = useState(access)
  const seed = initialAccess === access && initialViewerId === viewerId ? initial : null
  const resource = useCachedResource({
    identity, initialValue: seed?.revisions, load, resource: requestChangesResource,
    fallbackError: 'Request changes are unavailable.',
  })
  return { initial: seed, resource }
}
