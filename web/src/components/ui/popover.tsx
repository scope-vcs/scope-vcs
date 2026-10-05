import { cn } from '@/lib/utils'
import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'

type PopoverTriggerProps = {
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

export function Popover({
  align = 'end',
  className,
  label,
  panel,
  trigger,
}: {
  align?: keyof typeof ALIGN_CLASS | 'auto'
  className?: string
  label: string
  panel: (close: () => void) => ReactNode
  trigger: (props: PopoverTriggerProps) => ReactNode
}) {
  const [open, setOpen] = useState(false)
  const [flipped, setFlipped] = useState(false)
  const panelRef = useRef<HTMLDialogElement>(null)
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

  useLayoutEffect(() => {
    if (!open || align !== 'auto' || !panelRef.current || !triggerRef.current) return
    const left = triggerRef.current.getBoundingClientRect().left
    setFlipped(left + panelRef.current.offsetWidth > document.documentElement.clientWidth)
  }, [align, open])

  function close() {
    setOpen(false)
  }

  return (
    <div
      className="relative"
      onBlur={(event) => {
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
            ALIGN_CLASS[align === 'auto' ? (flipped ? 'end' : 'start') : align],
            className,
          )}
          id={panelId}
          open
          ref={panelRef}
        >
          {panel(close)}
        </dialog>
      )}
    </div>
  )
}
