import { ChevronDown } from 'lucide-react'
import { displayRouteFilePath } from '@/lib/route-file'
import { useId, useState, type CSSProperties, type ReactNode } from 'react'
import { PaneResizeHandle } from './pane-resize-handle'
import { cn } from '@/lib/utils'
import {
  clampFilePaneWidth,
  filePaneKeyboardWidth,
  FILE_PANE_MIN_WIDTH,
  FILE_PANE_MAX_WIDTH,
} from './file-workbench-width'

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
          {selectedPath ? displayRouteFilePath(selectedPath) : 'select a file'}
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
      <PaneResizeHandle
        className="hidden lg:block"
        controls={navigatorId}
        label="File pane width"
        max={FILE_PANE_MAX_WIDTH}
        min={FILE_PANE_MIN_WIDTH}
        onDrag={(distance) => setWidth(clampFilePaneWidth(width + distance))}
        onKey={(key) => {
          const next = filePaneKeyboardWidth(width, key)
          if (next === null) return false
          setWidth(next)
          return true
        }}
        width={width}
      />
      <div className="min-w-0">{content}</div>
    </div>
  )
}
