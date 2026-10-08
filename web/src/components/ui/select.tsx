import { cn } from '../../lib/utils'
import { ChevronDown } from 'lucide-react'
import type { ComponentProps } from 'react'

const SELECT_CLASS = 'w-full min-w-0 appearance-none truncate rounded-md border border-border bg-secondary text-secondary-foreground shadow-[var(--shadow-card)] outline-none transition-colors hover:bg-muted focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed'

const SIZE_CLASS = {
  compact: 'h-7 pl-2 pr-6 text-xs',
  default: 'h-8 pl-2.5 pr-7 text-sm',
} as const

const CHEVRON_CLASS = {
  compact: 'right-1.5 size-3',
  default: 'right-2 size-3.5',
} as const

export function Select({
  className,
  compact = false,
  ...props
}: Omit<ComponentProps<'select'>, 'size'> & { compact?: boolean }) {
  const size = compact ? 'compact' : 'default'
  return (
    <span className={cn('relative inline-flex min-w-0 has-disabled:opacity-45', className)}>
      <select className={cn(SELECT_CLASS, SIZE_CLASS[size])} {...props} />
      <ChevronDown
        aria-hidden="true"
        className={cn('pointer-events-none absolute top-1/2 -translate-y-1/2 text-muted-foreground', CHEVRON_CLASS[size])}
      />
    </span>
  )
}
