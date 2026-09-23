import { Clock3 } from 'lucide-react'
import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react'
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
  const id = useId()
  const trigger = useRef<HTMLButtonElement>(null)
  const menu = useRef<HTMLDivElement>(null)
  // The moment the menu opened; null while closed and on the server, so the
  // landing times never render into markup the browser has to reconcile.
  const [openedAt, setOpenedAt] = useState<Date | null>(null)

  useEffect(() => {
    if (!openedAt) return
    const hide = () => menu.current?.hidePopover()
    window.addEventListener('resize', hide)
    window.addEventListener('scroll', hide, true)
    return () => {
      window.removeEventListener('resize', hide)
      window.removeEventListener('scroll', hide, true)
    }
  }, [openedAt])

  function position() {
    if (!menu.current || !trigger.current) return
    const bounds = trigger.current.getBoundingClientRect()
    const { offsetWidth: width, offsetHeight: height } = menu.current
    const top =
      window.innerHeight - bounds.bottom >= height + 12
        ? bounds.bottom + 6
        : Math.max(8, bounds.top - height - 6)
    menu.current.style.top = `${top}px`
    menu.current.style.left = `${Math.max(8, Math.min(bounds.right - width, window.innerWidth - width - 8))}px`
    menu.current.querySelector('button')?.focus({ preventScroll: true })
  }

  return (
    <>
      <button
        aria-expanded={openedAt !== null}
        aria-haspopup="menu"
        aria-label={`Snooze request ${requestId}`}
        className="request-workspace-row-action"
        disabled={disabled}
        popoverTarget={id}
        ref={trigger}
        title={`Snooze request ${requestId}`}
        type="button"
      >
        <Clock3 aria-hidden="true" />
      </button>
      <div
        aria-label={`Snooze request ${requestId}`}
        className="fixed m-0 w-48 max-w-[calc(100vw-16px)] rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-[var(--shadow-pop)]"
        id={id}
        onKeyDown={moveFocus}
        onToggle={(event) => {
          const showing = event.newState === 'open'
          setOpenedAt(showing ? new Date() : null)
          if (showing) position()
        }}
        popover="auto"
        ref={menu}
        role="menu"
        tabIndex={-1}
      >
        <p className="px-2 pt-1 pb-0.5 text-[10px] text-muted-foreground">Snooze</p>
        {REQUEST_SNOOZE_OPTIONS.map((option) => (
          <button
            className="flex w-full items-baseline justify-between gap-3 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
            key={option.value}
            onClick={() => {
              menu.current?.hidePopover()
              onSnooze(requestSnoozeUntil(option.value))
            }}
            role="menuitem"
            type="button"
          >
            <span>{option.label}</span>
            <span className="font-mono text-[10px] text-muted-foreground tabular-nums">
              {openedAt && requestSnoozeLandingLabel(option.value, openedAt)}
            </span>
          </button>
        ))}
      </div>
    </>
  )
}

function moveFocus(event: KeyboardEvent<HTMLDivElement>) {
  const buttons = Array.from(event.currentTarget.querySelectorAll('button'))
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
  const next = {
    Home: 0,
    End: buttons.length - 1,
    ArrowDown: (index + 1) % buttons.length,
    ArrowUp: (index - 1 + buttons.length) % buttons.length,
  }[event.key]
  if (next === undefined) return
  event.preventDefault()
  buttons[next]?.focus({ preventScroll: true })
}
