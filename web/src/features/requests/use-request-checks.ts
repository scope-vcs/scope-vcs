import { useCallback, useState } from 'react'
import type { RequestChecksResponse } from '@/api/types.generated'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { reconcileRequestState, requestStateResource } from './request-state-resource'

export type RequestChecksController = {
  approve: (expectedHeadOid: string) => Promise<boolean>
  approving: boolean
  checks: RequestChecksResponse
  error: string | null
}

export function useRequestChecks({
  approve,
  identity,
  checks,
}: {
  approve: (expectedHeadOid: string) => Promise<RequestChecksResponse>
  identity: string
  checks: RequestChecksResponse
}): RequestChecksController {
  const [approving, setApproving] = useState(false)
  const [approveError, setApproveError] = useState<string | null>(null)

  const runApproval = useCallback(async (expectedHeadOid: string) => {
    const snapshot = requestStateResource.getSnapshot(identity)
    setApproving(true)
    setApproveError(null)
    try {
      const result = await approve(expectedHeadOid)
      reconcileRequestState(identity, snapshot, (state) => ({
        ...state,
        checks: result,
        detail: { request: { ...state.detail.request, mergeability: result.mergeability } },
        auto_merge: { ...state.auto_merge, can_enable: false },
      }))
      return true
    } catch (cause) {
      setApproveError(resourceErrorMessage(cause, 'CI could not be started.'))
      return false
    } finally {
      requestStateResource.invalidate(identity)
      setApproving(false)
    }
  }, [approve, identity])

  return {
    approve: runApproval,
    approving,
    checks,
    error: approveError,
  }
}
