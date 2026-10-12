import { useState } from 'react'
import { CirclePlay } from 'lucide-react'
import type { RequestChecksResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { requestChecksWorkflowWarning, requestPublicChecksNote } from './request-labels'
import type { RequestChecksController } from './use-request-checks'

export function RequestCiApproval({
  controller,
  requestViewName,
}: {
  controller: RequestChecksController
  requestViewName: string
}) {
  const [reviewed, setReviewed] = useState<RequestChecksResponse | null>(null)
  const warning = reviewed && requestChecksWorkflowWarning(reviewed)
  const publicNote = reviewed && requestPublicChecksNote(reviewed, requestViewName)

  return (
    <>
      {controller.checks.can_approve ? (
        <Button
          disabled={controller.approving}
          onClick={() => setReviewed(controller.checks)}
          size="sm"
          type="button"
          variant="secondary"
        >
          <CirclePlay />
          Allow CI to run
        </Button>
      ) : null}
      {reviewed ? (
        <RequestConfirmDialog
          confirmLabel="Allow CI to run"
          onConfirm={() => controller.approve(reviewed.head_oid)}
          onOpenChange={(open) => { if (!open) setReviewed(null) }}
          open
          pending={controller.approving}
          title="Allow CI to run?"
        >
          <p>Run CI for this revision. This does not approve or merge the contribution.</p>
          <p className="break-all font-mono text-xs text-foreground">{reviewed.head_oid}</p>
          {warning ? <p className="text-warning-strong">{warning}</p> : null}
          {publicNote ? <p className="text-warning-strong">{publicNote}</p> : null}
          {controller.error ? <p className="text-danger-strong" role="alert">{controller.error}</p> : null}
        </RequestConfirmDialog>
      ) : null}
    </>
  )
}
