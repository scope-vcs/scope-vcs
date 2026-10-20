import { ANCHORED_MENU_CLASS, useAnchoredMenu } from '@/components/ui/use-anchored-menu'
import { cn } from '@/lib/utils'
import { Clock3 } from 'lucide-react'
import { useState } from 'react'
import {
  REQUEST_SNOOZE_OPTIONS,
  requestSnoozeLandingLabel,
  requestSnoozeUntil,
} from './request-workspace-model'

export function RequestSnoozeMenu({
  requestId,
  disabled,
  onSnooze,
}: {
  requestId: string
  disabled: boolean
  onSnooze: (until: number) => void
}) {
  const [openedAt, setOpenedAt] = useState<Date | null>(null)
  const menu = useAnchoredMenu({
    align: 'end',
    onOpenChange: (open) => setOpenedAt(open ? new Date() : null),
  })

  return (
    <>
      <button
        {...menu.triggerProps}
        aria-label={`Snooze request ${requestId}`}
        className="request-workspace-row-action"
        disabled={disabled}
        title="Snooze"
        type="button"
      >
        <Clock3 aria-hidden="true" />
      </button>
      <div {...menu.menuProps} aria-label={`Snooze request ${requestId}`} className={cn(ANCHORED_MENU_CLASS, 'w-48')}>
        <p className="px-2 pt-1 pb-0.5 text-xs text-muted-foreground">Snooze</p>
        {REQUEST_SNOOZE_OPTIONS.map((option) => (
          <button
            className="flex w-full items-baseline justify-between gap-3 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
            key={option.value}
            onClick={() => {
              if (!openedAt) return
              const until = requestSnoozeUntil(option.value, openedAt)
              menu.hide()
              onSnooze(until)
            }}
            role="menuitem"
            type="button"
          >
            <span>{option.label}</span>
            <span className="text-xs text-muted-foreground tabular-nums">
              {openedAt && requestSnoozeLandingLabel(option.value, openedAt)}
            </span>
          </button>
        ))}
      </div>
    </>
  )
}
