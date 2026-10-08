import { useAuth } from '@clerk/tanstack-react-start'
import { useRouter } from '@tanstack/react-router'
import { useEffect } from 'react'
import { activateRequestAttachmentDraftViewer } from '@/features/requests/request-attachment-drafts'
import { activateViewer } from '@/lib/viewer-state'
export function ViewerSessionBoundary() {
  const { isLoaded, userId } = useAuth()
  const router = useRouter()
  useEffect(() => {
    if (!isLoaded) return
    const viewerId = userId ?? 'anonymous'
    const previous = activateViewer(userId ?? null)
    const viewerChanged = previous !== undefined && previous !== (userId ?? null)
    const firstSignedInViewer = previous === undefined && userId !== null
    if (viewerChanged) {
      activateRequestAttachmentDraftViewer(viewerId)
    }
    if (!viewerChanged && !firstSignedInViewer) return

    let active = true
    let pending = false
    const retry = () => {
      if (!active || pending) return
      pending = true
      void router.invalidate({ sync: true }).then(() => {
        if (!active) return
        if (router.state.matches.some((match) => match.status === 'error' || match.status === 'notFound')) return
        window.removeEventListener('focus', retry)
        window.removeEventListener('online', retry)
      }).catch(() => {}).finally(() => { pending = false })
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
