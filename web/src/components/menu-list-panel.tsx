import type { ReactNode } from 'react'

/** A row in a menu list: title and meta on the left, a count or note on the right. */
export const MENU_LIST_ROW_CLASS =
  'grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 px-3 py-2.5 text-left hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring'

/** The inside of a list popover: filters or search above a scrolling list. */
export function MenuListPanel({ children, controls }: { children: ReactNode; controls: ReactNode }) {
  return (
    <div className="text-xs">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border p-2">
        {controls}
      </div>
      <div className="max-h-[min(26rem,60vh)] overflow-y-auto">{children}</div>
    </div>
  )
}
