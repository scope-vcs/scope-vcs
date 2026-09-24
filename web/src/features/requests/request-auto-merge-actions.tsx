import type { RequestSummaryResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { shortOid } from '@/lib/short-oid'
import { Clock3, LoaderCircle, X } from 'lucide-react'
import { useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { autoMergeAuthorizer } from './request-auto-merge-model'
import { canMergeRequest } from './request-lifecycle-model'
import type { RequestAutoMergeController } from './use-request-auto-merge'

type Dialog =
  | {
    expectedHeadOid: string
    expectedRevisionId: string
    kind: 'authorize'
  }
  | {
    expectedIntentId: string
    headOid: string
    kind: 'cancel'
  }
  | null

export function RequestAutoMergeActions({
  autoMerge,
  disabled,
  onDialogOpenChange,
  request,
  viewerId,
}: {
  autoMerge: RequestAutoMergeController
  disabled: boolean
  onDialogOpenChange: (open: boolean) => void
  request: RequestSummaryResponse
  viewerId: string
}) {
  const [dialog, setDialog] = useState<Dialog>(null)
  const { status } = autoMerge
  const intent = status?.intent ?? null
  const revisionId = status?.revision_id ?? null
  const active = intent?.status === 'Active'
  const pending = autoMerge.pending !== null
  // Checks and auto-merge status refresh separately. Once the request can merge
  // directly, an offer that has not caught up yet would duplicate Merge.
  const canOffer = status?.can_enable === true && !canMergeRequest(request)

  function openDialog(next: Exclude<Dialog, null>) {
    setDialog(next)
    onDialogOpenChange(true)
  }

  function closeDialog(open: boolean) {
    if (open) return
    setDialog(null)
    onDialogOpenChange(false)
  }

  if (!status) return null

  return (
    <>
      {/* Ended authorizations stay in the activity history, not the header. */}
      {active ? (
        <span
          aria-live="polite"
          className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs"
        >
          <span className="inline-flex items-center gap-1.5 font-medium text-foreground">
            <Clock3 className="size-3.5 text-info-strong" />
            Will merge when checks pass
          </span>
          <span className="text-muted-foreground">
            {autoMergeAuthorizer(intent.actor, viewerId)} · {shortOid(intent.head_oid)}
            {status.waiting_reason ? ` · ${status.waiting_reason}` : ''}
          </span>
          {status.can_cancel ? (
            <Button
              aria-label="Cancel auto-merge"
              disabled={disabled || pending}
              onClick={() => openDialog({
                expectedIntentId: intent.id,
                headOid: intent.head_oid,
                kind: 'cancel',
              })}
              className="h-6 px-1.5 text-xs"
              size="sm"
              type="button"
              variant="ghost"
            >
              {autoMerge.pending === 'cancel'
                ? <LoaderCircle className="animate-spin" />
                : <X />}
              Cancel
            </Button>
          ) : null}
        </span>
      ) : null}
      {!active && canOffer && revisionId ? (
        <Button
          disabled={disabled || pending}
          onClick={() => openDialog({
            expectedHeadOid: status.head_oid,
            expectedRevisionId: revisionId,
            kind: 'authorize',
          })}
          size="sm"
          type="button"
          variant="success"
        >
          {autoMerge.pending === 'authorize'
            ? <LoaderCircle className="animate-spin" />
            : <Clock3 />}
          Merge when checks pass
        </Button>
      ) : null}

      {dialog?.kind === 'authorize' ? (
        <RequestConfirmDialog
          confirmLabel="Enable auto-merge"
          onConfirm={() => autoMerge.authorize({
            expected_head_oid: dialog.expectedHeadOid,
            expected_revision_id: dialog.expectedRevisionId,
          })}
          onOpenChange={closeDialog}
          open
          pending={autoMerge.pending === 'authorize'}
          title="Merge when checks pass?"
        >
          <p>
            Scope will merge “{request.title}” after this exact revision passes its checks.
            A new push ends this authorization.
          </p>
          <dl className="grid min-w-0 gap-1 font-mono text-xs">
            <div className="grid min-w-0 grid-cols-[4.5rem_minmax(0,1fr)] gap-2">
              <dt>Commit</dt>
              <dd className="break-all text-foreground">{dialog.expectedHeadOid}</dd>
            </div>
          </dl>
        </RequestConfirmDialog>
      ) : null}
      {dialog?.kind === 'cancel' ? (
        <RequestConfirmDialog
          confirmLabel="Cancel auto-merge"
          destructive
          onConfirm={() => autoMerge.cancel({
            expected_intent_id: dialog.expectedIntentId,
          })}
          onOpenChange={closeDialog}
          open
          pending={autoMerge.pending === 'cancel'}
          title="Cancel auto-merge?"
        >
          <p>
            Passing checks will leave this request open. Checks that are already running will continue.
          </p>
          <p className="font-mono text-xs">{shortOid(dialog.headOid)} → main</p>
        </RequestConfirmDialog>
      ) : null}
    </>
  )
}
