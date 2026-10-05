import { cn } from '@/lib/utils'

export const RUN_JOB_LIST_CLASS =
  'flex min-w-0 flex-1 gap-1 overflow-x-auto p-2 lg:flex-col lg:gap-0 lg:overflow-x-visible lg:p-0'

export const RUN_JOB_ITEM_CLASS = 'shrink-0'

export const RUN_JOB_ROW_CLASS =
  'relative flex h-9 max-w-64 shrink-0 items-center gap-2.5 rounded-md px-3 text-left text-sm lg:w-full lg:max-w-none lg:rounded-none lg:px-4'

export function runJobButtonClass(selected: boolean) {
  return cn(
    RUN_JOB_ROW_CLASS,
    'outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
    selected && 'bg-muted font-medium lg:before:absolute lg:before:inset-y-1.5 lg:before:left-0 lg:before:w-0.5 lg:before:rounded-full lg:before:bg-foreground',
  )
}
