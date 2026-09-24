import { useCallback, useState } from 'react'
import type { RequestChecksResponse } from '@/api/types.generated'
import {
  resourceErrorMessage,
  useCachedResource,
  useRetryOnReconnect,
} from '@/lib/use-cached-resource'
import { requestChecksResource } from './request-checks-resource'

export type RequestChecksController = {
  approve: () => Promise<void>
  approving: boolean
  checks: RequestChecksResponse | null
  error: string | null
}

// The resource owns the evaluation; approval answers with the refreshed one, so
// the result is written back instead of triggering another read.
export function useRequestChecks({
  approve,
  identity,
  load,
}: {
  approve: () => Promise<RequestChecksResponse>
  identity: string | null
  load: (signal: AbortSignal) => Promise<RequestChecksResponse>
}): RequestChecksController {
  const resource = useCachedResource({
    fallbackError: 'Request checks are unavailable.',
    identity,
    load,
    resource: requestChecksResource,
  })
  useRetryOnReconnect(resource)
  const [approving, setApproving] = useState(false)
  const [approveError, setApproveError] = useState<string | null>(null)

  const runApproval = useCallback(async () => {
    if (!identity) return
    setApproving(true)
    setApproveError(null)
    try {
      requestChecksResource.write(identity, await approve())
    } catch (cause) {
      setApproveError(resourceErrorMessage(cause, 'The checks could not be started.'))
    } finally {
      setApproving(false)
    }
  }, [approve, identity])

  return {
    approve: runApproval,
    approving,
    checks: resource.value,
    error: approveError ?? resource.error,
  }
}
