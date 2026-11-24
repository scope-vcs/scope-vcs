import type { RepoParams } from '@/api/types'
import type { RequestCloseReason, RequestSummaryResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { Link } from '@tanstack/react-router'
import { Code, Ellipsis, History } from 'lucide-react'
import { type RefObject, useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { REQUEST_CLOSE_REASONS, requestCloseReasonLabel } from './request-labels'
import { RequestOutcomeIcon } from './request-outcome-icon'
import type { RequestActionController } from './use-request-actions'

const ITEM_CLASS = 'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5'
const MENU_WIDTH_PX = 160
const VIEWPORT_EDGE_MARGIN_PX = 8
const MENU_CLEARANCE_PX = MENU_WIDTH_PX + VIEWPORT_EDGE_MARGIN_PX

export function RequestMoreMenu({
  actions,
  disabled,
  onViewActivity,
  params,
  request,
  triggerRef,
}: {
  actions: RequestActionController
  disabled: boolean
  onViewActivity: () => void
  params: RepoParams
  request: RequestSummaryResponse
  triggerRef: RefObject<HTMLButtonElement | null>
}) {
  const [closing, setClosing] = useState(false)
  const [reason, setReason] = useState<RequestCloseReason | null>(null)
  const [align, setAlign] = useState<'start' | 'end'>('end')
  const {
    can_close: canClose,
    can_read_change: canReadChange,
    can_view_activity: canViewActivity,
  } = request.permissions
  const needsReason = Boolean(request.ask)

  return (
    <>
      <Popover
        align={align}
        className="w-40 rounded-md p-1"
        label="More request actions"
        renderPanel={(close) => (
          <>
            {canReadChange ? (
              <Link
                className={ITEM_CLASS}
                onClick={close}
                params={params}
                search={{ request: request.id }}
                to="/$owner/$repo/code"
              >
                <Code aria-hidden="true" />
                Browse code
              </Link>
            ) : null}
            {canViewActivity ? (
              <button
                className={ITEM_CLASS}
                onClick={() => {
                  close()
                  onViewActivity()
                }}
                type="button"
              >
                <History aria-hidden="true" />
                View activity
              </button>
            ) : null}
            {canClose ? (
              <button
                className={`${ITEM_CLASS} text-danger`}
                disabled={disabled}
                onClick={() => {
                  close()
                  setClosing(true)
                }}
                type="button"
              >
                <RequestOutcomeIcon outcome="closed" />
                Close request
              </button>
            ) : null}
          </>
        )}
        renderTrigger={({ onClick, ref, ...props }) => (
          <Button
            aria-label="More request actions"
            onClick={(event) => {
              const { right } = event.currentTarget.getBoundingClientRect()
              setAlign(right < MENU_CLEARANCE_PX ? 'start' : 'end')
              onClick()
            }}
            ref={(element) => {
              ref.current = element
              triggerRef.current = element
            }}
            size="icon-sm"
            title="More request actions"
            type="button"
            variant="ghost"
            {...props}
          >
            <Ellipsis />
          </Button>
        )}
      />
      <RequestConfirmDialog
        confirmDisabled={needsReason && reason === null}
        confirmLabel="Close request"
        destructive
        onConfirm={() => actions.run(reason ? { action: 'close', reason } : { action: 'close' })}
        onOpenChange={setClosing}
        open={closing}
        pending={actions.pending === 'close'}
        returnFocus={triggerRef}
        title="Close this request?"
      >
        {needsReason ? (
          <>
            <p>This request will close and remain in request history.</p>
            <fieldset className="grid gap-1.5">
              <legend className="mb-1.5 text-foreground">Why is it closing?</legend>
              {REQUEST_CLOSE_REASONS.map((option) => (
                <label className="flex items-center gap-2 text-foreground" key={option}>
                  <input
                    checked={reason === option}
                    className="accent-foreground"
                    name="close-reason"
                    onChange={() => setReason(option)}
                    type="radio"
                    value={option}
                  />
                  <span className="first-letter:uppercase">{requestCloseReasonLabel(option)}</span>
                </label>
              ))}
            </fieldset>
          </>
        ) : request.submitted_at_unix === null ? (
          <p>This draft request will be permanently deleted.</p>
        ) : (
          <p>This submitted request will close and remain in request history.</p>
        )}
      </RequestConfirmDialog>
    </>
  )
}
