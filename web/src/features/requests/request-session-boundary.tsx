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
    const viewerChanged = activeViewer !== null && activeViewer !== viewerId
    const firstSignedInViewer = activeViewer === null && userId !== null
    if (viewerChanged) {
      activateRequestAttachmentDraftViewer(viewerId)
      resetRequestAttachmentMediaGrants()
      resetRequestAttachmentResources()
      resetRequestDiscussionCache()
      resetRequestMermaidResource()
      requestQueueResource.clear()
    }
    activeViewer = viewerId
    if (!viewerChanged && !firstSignedInViewer) return

    let active = true
    let pending = false
    const retry = () => {
      if (!active || pending) return
      pending = true
      void router.invalidate({ sync: true }).then(() => {
        if (!active) return
        // Router invalidation resolves after committing loader errors too.
        if (router.state.matches.some((match) => match.status === 'error' || match.status === 'notFound')) return
        window.removeEventListener('focus', retry)
        window.removeEventListener('online', retry)
      }).catch(() => {
        // Retain the listeners so an interrupted refresh can run again.
      }).finally(() => { pending = false })
    }
    window.addEventListener('focus', retry)
    window.addEventListener('online', retry)
    retry()
    return () => {
      active = false
      window.removeEventListener('focus', retry)
      window.removeEventListener('online', retry)
    }
  }, [isLoaded, router, userId])
  return null
}
