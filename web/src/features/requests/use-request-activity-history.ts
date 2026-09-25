import { useCallback, useState } from 'react'
import { useCachedResource } from '../../lib/use-cached-resource'
import { requestActivityResource } from './request-activity-resource'
import type { RequestActivityPage } from './request-discussion-types'

export function useRequestActivityHistory({
  identity,
  initialValue,
  load,
  version,
}: {
  identity: string | null
  initialValue: RequestActivityPage | null
  load: (signal: AbortSignal) => Promise<RequestActivityPage>
  version: string
}) {
  const [openIdentity, setOpenIdentity] = useState<string | null>(null)
  const open = identity !== null && openIdentity === identity
  // The discussion shows pushes from this activity, so it loads with the page.
  const resource = useCachedResource({
    fallbackError: 'Request history could not be loaded.',
    identity,
    initialValue,
    load,
    resource: requestActivityResource,
    version,
  })
  const openHistory = useCallback(() => setOpenIdentity(identity), [identity])
  const onOpenChange = useCallback((nextOpen: boolean) => {
    if (!nextOpen) setOpenIdentity(null)
  }, [])
  return {
    activity: resource.value,
    error: resource.error,
    loading: resource.status === 'loading',
    onOpenChange,
    open,
    openHistory,
    retry: resource.retry,
  }
}
