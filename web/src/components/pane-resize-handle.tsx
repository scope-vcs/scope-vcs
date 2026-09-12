import { useRef } from 'react'
import { cn } from '@/lib/utils'

export function PaneResizeHandle({
  label,
  controls,
  width,
  min,
  max,
  valueText,
  onDrag,
  onKey,
  className,
}: {
  label: string
  controls: string
  width: number
  min: number
  max: number
  valueText?: string
  onDrag: (distance: number) => void
  onKey: (key: string) => boolean
  className?: string
}) {
  const drag = useRef<{ x: number; resize: typeof onDrag } | null>(null)
  function resize(clientX: number) {
    if (drag.current) drag.current.resize(clientX - drag.current.x)
  }
  return (
    <button
      aria-controls={controls}
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemax={max}
      aria-valuemin={min}
      aria-valuenow={width}
      aria-valuetext={valueText}
      className={cn(
        'relative z-10 m-0 h-auto w-px self-stretch border-0 bg-border cursor-col-resize touch-none before:absolute before:inset-y-0 before:-left-1 before:w-2 hover:bg-brand focus-visible:bg-brand focus-visible:outline-2 focus-visible:outline-ring',
        className,
      )}
      onKeyDown={(event) => {
        if (onKey(event.key)) event.preventDefault()
      }}
      onLostPointerCapture={() => {
        drag.current = null
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return
        event.preventDefault()
        event.currentTarget.focus()
        event.currentTarget.setPointerCapture(event.pointerId)
        drag.current = { x: event.clientX, resize: onDrag }
      }}
      onPointerMove={(event) => resize(event.clientX)}
      onPointerUp={(event) => {
        resize(event.clientX)
        drag.current = null
        if (event.currentTarget.hasPointerCapture(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId)
      }}
      role="separator"
      type="button"
    />
  )
}
