import { ChevronDown } from 'lucide-react'
import { useId, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { clampFilePaneWidth, filePaneKeyboardWidth } from './file-workbench-width'

export function FileWorkbench({
  children,
  selectedPath,
  navigationOpen,
  onNavigationOpenChange,
  className,
}: {
  children: [ReactNode, ReactNode]
  selectedPath: string | null
  navigationOpen: boolean
  onNavigationOpenChange: (open: boolean) => void
  className?: string
}) {
  const [navigator, content] = children
  const [width, setWidth] = useState(250)
  const drag = useRef<{ x: number; width: number } | null>(null)
  const navigatorId = useId()
  return (
    <div
      className={cn(
        'min-w-0 lg:grid lg:grid-cols-[var(--file-pane-width)_1px_minmax(0,1fr)]',
        className,
      )}
      style={{ '--file-pane-width': `${width}px` } as CSSProperties}
    >
      <button
        aria-controls={navigatorId}
        aria-expanded={navigationOpen}
        className="flex w-full min-w-0 items-center gap-2 border-b border-border px-4 py-3 text-left text-xs lg:hidden"
        onClick={() => onNavigationOpenChange(!navigationOpen)}
        type="button"
      >
        <ChevronDown
          className={cn('size-4 shrink-0 transition-transform', navigationOpen && 'rotate-180')}
        />
        <span className="shrink-0">files</span>
        <span className="min-w-0 break-all font-mono text-muted-foreground">
          {selectedPath?.replace(/^\//, '') ?? 'select a file'}
        </span>
      </button>
      <div
        className={cn(
          'min-w-0 overflow-x-auto border-b border-border lg:block lg:border-b-0',
          !navigationOpen && 'hidden',
        )}
        id={navigatorId}
      >
        {navigator}
      </div>
      <button
        type="button"
        aria-label="File pane width"
        aria-controls={navigatorId}
        aria-orientation="vertical"
        aria-valuemax={360}
        aria-valuemin={180}
        aria-valuenow={width}
        className="relative z-10 m-0 hidden h-auto w-px self-stretch border-0 cursor-col-resize touch-none bg-border before:absolute before:inset-y-0 before:-left-1 before:w-2 hover:bg-brand focus-visible:bg-brand focus-visible:outline-2 focus-visible:outline-ring lg:block"
        onKeyDown={(event) => {
          const nextWidth = filePaneKeyboardWidth(width, event.key)
          if (nextWidth === null) return
          event.preventDefault()
          setWidth(nextWidth)
        }}
        onPointerDown={(event) => {
          if (event.button !== 0) return
          event.preventDefault()
          event.currentTarget.focus()
          event.currentTarget.setPointerCapture(event.pointerId)
          drag.current = { x: event.clientX, width }
        }}
        onPointerMove={(event) => {
          if (drag.current) {
            setWidth(clampFilePaneWidth(drag.current.width + event.clientX - drag.current.x))
          }
        }}
        onPointerUp={(event) => {
          drag.current = null
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId)
          }
        }}
        onLostPointerCapture={() => {
          drag.current = null
        }}
        role="separator"
        tabIndex={0}
      />
      <div className="min-w-0">{content}</div>
    </div>
  )
}
