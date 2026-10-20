import { cn } from '@/lib/utils'

export const RUN_STEP_ROW_CLASS =
  'grid h-9 w-full grid-cols-[auto_auto_minmax(0,1fr)_auto] items-center gap-2 px-4'

export const RUN_ROW_CLASS = 'flex min-w-0 items-center gap-3 px-3 py-3'
export const RUN_ROW_DURATION_CLASS = 'w-16 shrink-0'
export const RUN_ROW_TIMESTAMP_CLASS = 'w-24 shrink-0'
export const RUN_HISTORY_ROW_CLASS =
  'grid grid-cols-[auto_minmax(0,max-content)_auto_minmax(0,1fr)] items-center gap-y-0.5 px-3 py-3 sm:grid-cols-[auto_minmax(0,1fr)_auto_auto] sm:gap-x-3'
export const RUN_HISTORY_ROW_ICON_CLASS = 'col-start-1 row-span-2 row-start-1 mr-3 sm:row-span-1 sm:mr-0'
export const RUN_HISTORY_ROW_PRIMARY_CLASS = 'contents sm:flex sm:min-w-0 sm:items-baseline sm:gap-2'
export const RUN_HISTORY_ROW_FIGURE_CLASS =
  "row-start-2 whitespace-nowrap max-sm:w-auto sm:row-start-1 text-xs tabular-nums text-muted-foreground before:px-1 before:text-muted-foreground/70 before:content-['·'] sm:text-right sm:before:content-none"
export const RUN_ROW_WIDE_COLUMN_CLASS = 'hidden sm:block'

export const RUN_LINK_CLASS = 'underline-offset-2 hover:text-foreground hover:underline'

export const RUN_LOG_TEXT_CLASS = 'bg-background font-mono text-xs leading-5 text-foreground'

export function runLogPreClass(wrap: boolean) {
  return cn('mt-1 overflow-x-auto break-words pb-4', wrap ? 'whitespace-pre-wrap' : 'whitespace-pre')
}

export const RUN_JOB_LIST_CLASS =
  'flex min-w-0 flex-1 flex-wrap gap-1 p-2 lg:flex-col lg:flex-nowrap lg:gap-0 lg:p-0'

export const RUN_JOB_ITEM_CLASS = 'max-w-full shrink-0'

export const RUN_JOB_ROW_CLASS =
  'relative flex h-9 max-w-64 shrink-0 items-center gap-2.5 rounded-md px-3 text-left text-sm lg:w-full lg:max-w-none lg:rounded-none lg:px-4'

export function runJobButtonClass(selected: boolean) {
  return cn(
    RUN_JOB_ROW_CLASS,
    'outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
    selected && 'bg-muted font-medium lg:before:absolute lg:before:inset-y-1.5 lg:before:left-0 lg:before:w-0.5 lg:before:rounded-full lg:before:bg-foreground',
  )
}
