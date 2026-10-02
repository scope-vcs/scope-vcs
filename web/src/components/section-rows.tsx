import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

export function SectionRows({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <div className={cn('mt-6 divide-y divide-border', className)}>
      {children}
    </div>
  )
}

/** `id` lets other pages link to the section. */
export function SectionRow({
  children,
  description,
  icon,
  id,
  title,
}: {
  children: ReactNode
  description?: ReactNode
  icon?: ReactNode
  id?: string
  title: ReactNode
}) {
  return (
    <section className="grid scroll-mt-20 gap-4 py-5 md:grid-cols-[240px_minmax(0,1fr)]" id={id}>
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm font-semibold leading-5">
          {icon}
          <span>{title}</span>
        </div>
        {description && (
          <p className="mt-1 text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      <div className="min-w-0 md:pt-0.5">{children}</div>
    </section>
  )
}
