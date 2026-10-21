import { cn } from '../../lib/utils'
import { ChevronDown } from 'lucide-react'
import type { ComponentProps } from 'react'

const SELECT_CLASS = 'h-8 w-full min-w-0 appearance-none truncate rounded-md border border-border bg-secondary pl-2.5 pr-7 text-sm text-secondary-foreground shadow-[var(--shadow-card)] outline-none transition-colors hover:bg-muted focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed'

export function Select({ className, ...props }: Omit<ComponentProps<'select'>, 'size'>) {
  return (
    <span className={cn('relative inline-flex min-w-0 has-disabled:opacity-45', className)}>
      <select className={SELECT_CLASS} {...props} />
      <ChevronDown
        aria-hidden="true"
        className="pointer-events-none absolute top-1/2 right-2 size-3.5 -translate-y-1/2 text-muted-foreground"
      />
    </span>
  )
}
