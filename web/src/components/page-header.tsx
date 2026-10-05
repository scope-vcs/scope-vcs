import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

type RailElement = 'div' | 'header' | 'main' | 'section'
type RailProps = {
  as?: RailElement
  children: ReactNode
  className?: string
  id?: string
  tabIndex?: number
}

function AppRail({
  as: Component = 'div',
  children,
  className,
  id,
  tabIndex,
}: RailProps) {
  return (
    <Component
      className={cn('mx-auto w-full max-w-[1280px]', className)}
      id={id}
      tabIndex={tabIndex}
    >
      {children}
    </Component>
  )
}

export function PageRail({
  as,
  children,
  className,
  id,
  tabIndex,
}: RailProps) {
  return (
    <AppRail
      as={as}
      className={cn('px-5 sm:px-6 lg:px-8', className)}
      id={id}
      tabIndex={tabIndex}
    >
      {children}
    </AppRail>
  )
}

export function PageContent({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <PageRail
      as="section"
      className={cn('scope-content-enter py-8 lg:py-10', className)}
    >
      {children}
    </PageRail>
  )
}

export function WorkbenchPane({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <AppRail className={cn('scope-content-enter', className)}>
      {children}
    </AppRail>
  )
}

export function PageHeader({
  actions,
  badges,
  children,
  description,
  title,
}: {
  actions?: ReactNode
  badges?: ReactNode
  children?: ReactNode
  description?: ReactNode
  title: ReactNode
  }) {
  return (
    <header className="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <h1 className="break-words text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          {title}
        </h1>
        {description && (
          <p className="mt-2 max-w-[62ch] text-[15px] leading-6 text-muted-foreground">
            {description}
          </p>
        )}
        {badges && (
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {badges}
          </div>
        )}
        {children}
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">
          {actions}
        </div>
      )}
    </header>
  )
}

export function WorkbenchBar({
  actions,
  className,
  summary,
  title,
}: {
  actions?: ReactNode
  className?: string
  summary?: ReactNode
  title: ReactNode
}) {
  return (
    <div
      className={cn(
        'flex min-h-14 flex-wrap items-center justify-between gap-x-4 gap-y-2 px-5 py-3 sm:px-6 lg:px-8',
        className,
      )}
    >
      <h1 className="sr-only">{title}</h1>
      {summary && (
        <div className="min-w-0 text-sm text-muted-foreground">{summary}</div>
      )}
      {actions && (
        <div className="ml-auto flex shrink-0 flex-wrap items-center gap-2">
          {actions}
        </div>
      )}
    </div>
  )
}
