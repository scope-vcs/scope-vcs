import { Button } from '@/components/ui/button'
import { shortOid } from '@/lib/short-oid'
import { cn } from '@/lib/utils'
import { CheckCircle2 } from 'lucide-react'
import { type Ref, useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { RequestAutoMergeActions } from './request-auto-merge-actions'
import {
  canMergeRequest,
  checksHoldRequestMerge,
  hasRequestAutoMergeActions,
  hasRequestLifecycleActions,
} from './request-lifecycle-model'
import type { RequestActionController } from './use-request-actions'
import type { RequestAutoMergeController } from './use-request-auto-merge'
import type { RequestSummaryResponse } from '@/api/types.generated'

type Dialog = 'merge' | 'submit' | null

export function RequestLifecycleActions({
  actions,
  autoMerge,
  className,
  ref,
  request,
  viewerId,
}: {
  actions: RequestActionController
  autoMerge: RequestAutoMergeController
  className?: string
  ref?: Ref<HTMLDivElement>
  request: RequestSummaryResponse
  viewerId: string
}) {
  const [dialog, setDialog] = useState<Dialog>(null)
  const [autoMergeDialogOpen, setAutoMergeDialogOpen] = useState(false)
  const busy = actions.pending !== null || autoMerge.pending !== null
  const permissions = request.permissions
  const canMerge = canMergeRequest(request)
  const checksHoldMerge = checksHoldRequestMerge(request)
  const publicRequest = request.author_role === 'Public'
  const submitLabel = publicRequest ? 'Request review' : 'Mark ready'

  const hasAutoMergeAction = hasRequestAutoMergeActions(
    autoMerge.status,
    autoMergeDialogOpen,
  )

  if (!hasRequestLifecycleActions(request) && !hasAutoMergeAction) return null

  return (
    <>
      <div className={cn('flex flex-wrap items-center gap-2', className)} ref={ref}>
        {permissions.can_submit ? (
          <Button disabled={busy} onClick={() => setDialog('submit')} size="sm" type="button">
            <CheckCircle2 />
            {submitLabel}
          </Button>
        ) : null}
        {canMerge ? (
          <Button disabled={busy} onClick={() => setDialog('merge')} size="sm" type="button" variant="success">
            Merge
          </Button>
        ) : null}
        <RequestAutoMergeActions
          autoMerge={autoMerge}
          disabled={busy}
          onDialogOpenChange={setAutoMergeDialogOpen}
          request={request}
          viewerId={viewerId}
        />
        {checksHoldMerge && !autoMerge.status?.can_enable &&
          autoMerge.status?.intent?.status !== 'Active' ? (
          <Button disabled size="sm" type="button" variant="success">
            Merge
          </Button>
        ) : null}
      </div>

      <RequestConfirmDialog
        confirmLabel={submitLabel}
        onConfirm={() => actions.run({ action: 'submit' })}
        onOpenChange={(open) => setDialog(open ? 'submit' : null)}
        open={dialog === 'submit'}
        pending={actions.pending === 'submit'}
        title={publicRequest ? 'Request maintainer review?' : 'Mark request ready?'}
      >
        <p>
          {publicRequest
            ? 'Send the current request to the repository maintainers for review. You can keep editing and pushing afterward.'
            : 'Mark the current maintainer request ready to merge. You can keep editing and pushing afterward.'}
        </p>
      </RequestConfirmDialog>
      <RequestConfirmDialog
        confirmLabel="Merge request"
        onConfirm={() => actions.run({ action: 'merge' })}
        onOpenChange={(open) => setDialog(open ? 'merge' : null)}
        open={dialog === 'merge'}
        pending={actions.pending === 'merge'}
        title="Merge this request?"
      >
        <p>This completes “{request.title}” and merges its current head into main.</p>
        <p className="font-mono text-xs">
          {shortOid(request.head_oid)} → main
        </p>
      </RequestConfirmDialog>
    </>
  )
}
