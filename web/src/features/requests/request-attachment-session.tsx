import { useAuth } from '@clerk/tanstack-react-start'
import { useEffect } from 'react'
import { activateRequestAttachmentDraftViewer } from './request-attachment-drafts'
import { resetRequestAttachmentMediaGrants } from './request-attachment-media-resource'
import { resetRequestAttachmentResources } from './request-attachment-resource'

let activeViewer: string | null = null

export function RequestAttachmentSessionBoundary() {
  const { isLoaded, userId } = useAuth()
  useEffect(() => {
    if (!isLoaded) return
    const viewerId = userId ?? 'anonymous'
    if (activeViewer !== null && activeViewer !== viewerId) {
      activateRequestAttachmentDraftViewer(viewerId)
      resetRequestAttachmentMediaGrants()
      resetRequestAttachmentResources()
    }
    activeViewer = viewerId
  }, [isLoaded, userId])
  return null
}
