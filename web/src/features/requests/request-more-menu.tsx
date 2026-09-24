import type { RequestSummaryResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { Ellipsis, History, XCircle } from 'lucide-react'
import { useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import type { RequestActionController } from './use-request-actions'

const ITEM_CLASS = 'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5'

/** The request's rare actions, kept out of the header's main action row. */
export function RequestMoreMenu({
  actions,
  disabled,
  onViewActivity,
  request,
}: {
  actions: RequestActionController
  disabled: boolean
  onViewActivity: () => void
  request: RequestSummaryResponse
}) {
  const [closing, setClosing] = useState(false)
  const { can_close: canClose, can_view_activity: canViewActivity } = request.permissions

  if (!canClose && !canViewActivity) return null

  return (
    <>
      <Popover
        className="request-detail-more-menu w-40 rounded-md p-1"
        label="More request actions"
        panel={(close) => (
          <>
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
                <XCircle aria-hidden="true" />
                Close request
              </button>
            ) : null}
          </>
        )}
        trigger={(props) => (
          <Button
            aria-label="More request actions"
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
        confirmLabel="Close request"
        destructive
        onConfirm={() => actions.run({ action: 'close' })}
        onOpenChange={setClosing}
        open={closing}
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
