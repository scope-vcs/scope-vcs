import { useAuth } from '@clerk/tanstack-react-start'
import { useEffect } from 'react'
import { activateRequestAttachmentDraftViewer } from './request-attachment-drafts'
import { resetRequestAttachmentMediaGrants } from './request-attachment-media-resource'
import { resetRequestAttachmentResources } from './request-attachment-resource'
import { resetRequestDiscussionCache } from './request-discussion-cache'
import { resetRequestQueueCache } from './request-queue-cache'

let activeViewer: string | null = null

// Request caches are keyed by viewer, but a viewer change still discards
// everything the previous viewer loaded so nothing of theirs lingers in memory.
export function RequestSessionBoundary() {
  const { isLoaded, userId } = useAuth()
  useEffect(() => {
    if (!isLoaded) return
    const viewerId = userId ?? 'anonymous'
    if (activeViewer !== null && activeViewer !== viewerId) {
      activateRequestAttachmentDraftViewer(viewerId)
      resetRequestAttachmentMediaGrants()
      resetRequestAttachmentResources()
      resetRequestDiscussionCache()
      resetRequestQueueCache()
    }
    activeViewer = viewerId
  }, [isLoaded, userId])
  return null
}
