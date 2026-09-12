import { Button } from '@/components/ui/button'
import { shortOid } from '@/lib/short-oid'
import { cn } from '@/lib/utils'
import { CheckCircle2, XCircle } from 'lucide-react'
import { useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { canMergeRequest, hasRequestLifecycleActions } from './request-lifecycle-model'
import type { RequestActionController } from './use-request-actions'
import type { RequestSummaryResponse } from '@/api/types.generated'

type Dialog = 'close' | 'merge' | 'submit' | null

export function RequestLifecycleActions({
  actions,
  className,
  request,
}: {
  actions: RequestActionController
  className?: string
  request: RequestSummaryResponse
}) {
  const [dialog, setDialog] = useState<Dialog>(null)
  const busy = actions.pending !== null
  const permissions = request.permissions
  const canMerge = canMergeRequest(request)
  const publicRequest = request.author_role === 'Public'
  const submitLabel = publicRequest ? 'Request review' : 'Mark ready'

  if (!hasRequestLifecycleActions(request)) return null

  return (
    <>
      <div className={cn('flex flex-wrap items-center gap-2', className)}>
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
        {permissions.can_close ? (
          <Button disabled={busy} onClick={() => setDialog('close')} size="sm" type="button" variant="destructive">
            <XCircle />
            Close
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
      <RequestConfirmDialog
        confirmLabel="Close request"
        destructive
        onConfirm={() => actions.run({ action: 'close' })}
        onOpenChange={(open) => setDialog(open ? 'close' : null)}
        open={dialog === 'close'}
        pending={actions.pending === 'close'}
        title="Close this request?"
      >
        {request.submitted_at_unix === null ? (
          <p>This draft request will be permanently deleted.</p>
        ) : (
          <p>This submitted request will close and remain in request history.</p>
        )}
      </RequestConfirmDialog>
    </>
  )
}
