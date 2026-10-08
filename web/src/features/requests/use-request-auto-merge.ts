import type { RequestAutoMergeResponse } from '@/api/types.generated'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { useCallback, useState } from 'react'
import type {
  AuthorizeRequestAutoMergeInput,
  CancelRequestAutoMergeInput,
} from './request-auto-merge-api'
import { reconcileRequestState, requestStateResource } from './request-state-resource'

type AuthorizeInput = Pick<
  AuthorizeRequestAutoMergeInput,
  'expected_head_oid' | 'expected_revision_id'
>
type CancelInput = Pick<CancelRequestAutoMergeInput, 'expected_intent_id'>

export type RequestAutoMergeController = {
  authorize: (input: AuthorizeInput) => Promise<boolean>
  cancel: (input: CancelInput) => Promise<boolean>
  error: string | null
  pending: 'authorize' | 'cancel' | null
  status: RequestAutoMergeResponse
}

export function useRequestAutoMerge({
  authorize,
  cancel,
  identity,
  status,
}: {
  authorize: (input: AuthorizeInput) => Promise<RequestAutoMergeResponse>
  cancel: (input: CancelInput) => Promise<RequestAutoMergeResponse>
  identity: string
  status: RequestAutoMergeResponse
}): RequestAutoMergeController {
  const [pending, setPending] = useState<'authorize' | 'cancel' | null>(null)
  const [mutationError, setMutationError] = useState<string | null>(null)

  const run = useCallback(async (
    action: 'authorize' | 'cancel',
    mutate: () => Promise<RequestAutoMergeResponse>,
  ) => {
    const snapshot = requestStateResource.getSnapshot(identity)
    setPending(action)
    setMutationError(null)
    try {
      const result = await mutate()
      reconcileRequestState(identity, snapshot, (state) => ({ ...state, auto_merge: result }))
      return true
    } catch (cause) {
      setMutationError(resourceErrorMessage(
        cause,
        action === 'authorize'
          ? 'Auto-merge could not be enabled.'
          : 'Auto-merge could not be canceled.',
      ))
      return false
    } finally {
      requestStateResource.invalidate(identity)
      setPending(null)
    }
  }, [identity])

  const runAuthorize = useCallback(
    (input: AuthorizeInput) => run('authorize', () => authorize(input)),
    [authorize, run],
  )
  const runCancel = useCallback(
    (input: CancelInput) => run('cancel', () => cancel(input)),
    [cancel, run],
  )

  return {
    authorize: runAuthorize,
    cancel: runCancel,
    error: mutationError,
    pending,
    status,
  }
}
