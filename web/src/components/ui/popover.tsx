import { cn } from '@/lib/utils'
import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from 'react'

export type PopoverTriggerProps = {
  'aria-controls': string
  'aria-expanded': boolean
  'aria-haspopup': 'dialog'
  onClick: () => void
  ref: RefObject<HTMLButtonElement | null>
}

const ALIGN_CLASS = {
  center: 'left-1/2 right-auto -translate-x-1/2',
  end: 'left-auto right-0',
  start: 'left-0 right-auto',
} as const

/**
 * A click-toggled panel anchored under its trigger. One dismissal contract for
 * every popup in the app: a pointer outside, Escape (focus returns to the
 * trigger), or keyboard focus leaving the panel all close it.
 */
export function Popover({
  align = 'end',
  className,
  label,
  panel,
  trigger,
}: {
  align?: keyof typeof ALIGN_CLASS
  className?: string
  label: string
  panel: (close: () => void) => ReactNode
  trigger: (props: PopoverTriggerProps) => ReactNode
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const panelId = useId()

  useEffect(() => {
    if (!open) return
    function closeOnOutsidePointer(event: PointerEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      setOpen(false)
      triggerRef.current?.focus()
    }
    document.addEventListener('pointerdown', closeOnOutsidePointer)
    document.addEventListener('keydown', closeOnEscape)
    return () => {
      document.removeEventListener('pointerdown', closeOnOutsidePointer)
      document.removeEventListener('keydown', closeOnEscape)
    }
  }, [open])

  function close() {
    setOpen(false)
  }

  return (
    <div
      className="relative"
      onBlur={(event) => {
        // A null relatedTarget is a pointer on non-focusable content, which the
        // document listener already handles; only a real focus move closes here.
        const next = event.relatedTarget
        if (open && next instanceof Node && !rootRef.current?.contains(next)) close()
      }}
      ref={rootRef}
    >
      {trigger({
        'aria-controls': panelId,
        'aria-expanded': open,
        'aria-haspopup': 'dialog',
        onClick: () => setOpen((value) => !value),
        ref: triggerRef,
      })}
      {open && (
        <dialog
          aria-label={label}
          className={cn(
            'absolute top-full z-50 m-0 mt-1 max-w-none rounded-lg border border-border bg-popover p-3 text-popover-foreground shadow-[var(--shadow-pop)]',
            ALIGN_CLASS[align],
            className,
          )}
          id={panelId}
          open
        >
          {panel(close)}
        </dialog>
      )}
    </div>
  )
}
