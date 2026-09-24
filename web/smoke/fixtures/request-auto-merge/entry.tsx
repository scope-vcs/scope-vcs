import type {
  RequestAutoMergeResponse,
  RequestSummaryResponse,
} from '@/api/types.generated'
import { RequestLifecycleActions } from '@/features/requests/request-lifecycle-actions'
import type { RequestActionController } from '@/features/requests/use-request-actions'
import type { RequestAutoMergeController } from '@/features/requests/use-request-auto-merge'
import { createRoot } from 'react-dom/client'
import { useState } from 'react'
import './styles.css'

const headOid = 'a'.repeat(40)
const revisionId = 'revision-7d9f2f1b'
const calls: unknown[] = []
Object.assign(window, { calls })

const request = {
  author_role: 'Member',
  head_oid: headOid,
  mergeability: {
    reason: 'Checks are still running.',
    status: 'ChecksPending',
  },
  permissions: {
    can_close: false,
    can_merge: true,
    can_submit: false,
  },
  title: 'Handle quoted paths',
} as RequestSummaryResponse

const requestActions: RequestActionController = {
  error: null,
  pending: null,
  run: async () => false,
}

function App() {
  const [loadingOnly, setLoadingOnly] = useState(false)
  const [status, setStatus] = useState<RequestAutoMergeResponse | null>({
    can_cancel: false,
    can_enable: true,
    head_oid: headOid,
    intent: null,
    request_id: 'request',
    revision_id: revisionId,
    waiting_reason: 'Checks are still running.',
  })
  Object.assign(window, {
    refreshAutoMerge: () => setStatus((current) => current && ({
      ...current,
      can_enable: false,
      head_oid: 'b'.repeat(40),
      intent: current.intent && {
        ...current.intent,
        head_oid: 'b'.repeat(40),
        id: 'intent-new',
        revision_id: 'revision-new',
      },
      revision_id: 'revision-new',
    })),
    setAutoMergeIntentStatus: (
      intentStatus: 'Cancelled' | 'Stopped',
    ) => setStatus((current) => current && ({
      ...current,
      can_cancel: false,
      can_enable: true,
      intent: current.intent && {
        ...current.intent,
        reason: intentStatus === 'Stopped' ? 'ChecksFailed' : null,
        status: intentStatus,
      },
      waiting_reason: null,
    })),
    showAutoMergeLoading: () => {
      setLoadingOnly(true)
      setStatus(null)
    },
  })
  const autoMerge: RequestAutoMergeController = {
    authorize: async (input) => {
      calls.push(input)
      setStatus({
        ...status!,
        can_cancel: true,
        can_enable: false,
        intent: {
          actor: { handle: 'adam', id: 'viewer' },
          created_at_unix: 1,
          head_oid: headOid,
          id: 'intent-1',
          reason: null,
          revision_id: revisionId,
          status: 'Active',
          updated_at_unix: 1,
        },
        waiting_reason: 'Checks are still running.',
      })
      return true
    },
    cancel: async (input) => {
      calls.push(input)
      setStatus({
        ...status!,
        can_cancel: false,
        can_enable: true,
        intent: status!.intent && { ...status!.intent, status: 'Cancelled' },
        waiting_reason: null,
      })
      return true
    },
    error: null,
    pending: null,
    status,
  }

  return (
    <main className="min-h-screen bg-background p-5 text-foreground">
      <h1 className="text-xl font-medium">Handle quoted paths</h1>
      <p className="mt-2 text-sm text-muted-foreground">Request fixture</p>
      <RequestLifecycleActions
        actions={requestActions}
        autoMerge={autoMerge}
        className="fixed inset-x-0 bottom-0 z-30 justify-end border-t border-border bg-background px-3 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] min-[701px]:static min-[701px]:mt-6 min-[701px]:justify-start min-[701px]:border-0 min-[701px]:p-0"
        request={loadingOnly ? {
          ...request,
          permissions: { ...request.permissions, can_merge: false },
        } : request}
        viewerId="viewer"
      />
    </main>
  )
}

createRoot(document.getElementById('root')!).render(<App />)
