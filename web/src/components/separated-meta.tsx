import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export function SeparatedMeta({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn('overflow-hidden', className)}>
      <p className="-ml-4 flex flex-wrap items-center gap-y-1">{children}</p>
    </div>
  )
}

export function SeparatedMetaItem({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "flex min-w-0 max-w-full items-center whitespace-nowrap before:w-4 before:shrink-0 before:text-center before:font-sans before:opacity-50 before:content-['·'_/_'']",
        className,
      )}
    >
      <span className="min-w-0 truncate">{children}</span>
    </span>
  )
}
