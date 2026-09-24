import { useAuth } from '@clerk/tanstack-react-start'
import { useRouter } from '@tanstack/react-router'
import { useEffect } from 'react'
import { activateAccountSessionViewer } from '../account/account-session-resource'
import { activateRequestAttachmentDraftViewer } from './request-attachment-drafts'
import { resetRequestAttachmentMediaGrants } from './request-attachment-media-resource'
import { resetRequestAttachmentResources } from './request-attachment-resource'
import { resetRequestDiscussionCache } from './request-discussion-cache'
import { resetRequestMermaidResource } from './request-mermaid-resource'
import { requestQueueResource } from './request-queue-cache'

let activeViewer: string | null = null

// Request caches are keyed by viewer, but a viewer change still discards
// everything the previous viewer loaded so nothing of theirs lingers in memory.
export function RequestSessionBoundary() {
  const { isLoaded, userId } = useAuth()
  const router = useRouter()
  useEffect(() => {
    if (!isLoaded) return
    const viewerId = userId ?? 'anonymous'
    activateAccountSessionViewer(viewerId)
    if (activeViewer !== null && activeViewer !== viewerId) {
      activateRequestAttachmentDraftViewer(viewerId)
      resetRequestAttachmentMediaGrants()
      resetRequestAttachmentResources()
      resetRequestDiscussionCache()
      resetRequestMermaidResource()
      requestQueueResource.clear()
      void router.invalidate().catch(() => {})
    }
    activeViewer = viewerId
  }, [isLoaded, router, userId])
  return null
}
