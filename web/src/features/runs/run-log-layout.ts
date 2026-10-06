import { cn } from '@/lib/utils'

export const RUN_LOG_TEXT_CLASS = 'bg-background font-mono text-xs leading-5 text-foreground'

export function runLogPreClass(wrap: boolean) {
  return cn(
    'mt-1 overflow-x-auto break-words pb-4',
    wrap ? 'whitespace-pre-wrap' : 'whitespace-pre',
  )
}
