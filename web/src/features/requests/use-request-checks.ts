import { useCallback, useState } from 'react'
import type { RequestChecksResponse } from '@/api/types.generated'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { reconcileRequestState, requestStateResource } from './request-state-resource'

export type RequestChecksController = {
  approve: () => Promise<void>
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

  const shownHeadOid = checks.head_oid
  const runApproval = useCallback(async () => {
    const snapshot = requestStateResource.getSnapshot(identity)
    setApproving(true)
    setApproveError(null)
    try {
      const result = await approve(shownHeadOid)
      reconcileRequestState(identity, snapshot, (state) => ({
        ...state,
        checks: result,
        detail: { request: { ...state.detail.request, mergeability: result.mergeability } },
        auto_merge: { ...state.auto_merge, can_enable: false },
      }))
    } catch (cause) {
      setApproveError(resourceErrorMessage(cause, 'The checks could not be started.'))
    } finally {
      requestStateResource.invalidate(identity)
      setApproving(false)
    }
  }, [approve, identity, shownHeadOid])

  return {
    approve: runApproval,
    approving,
    checks,
    error: approveError,
  }
}
