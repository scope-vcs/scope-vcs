import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageRail } from '@/components/page-header'
import {
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { useEffect, useState, type ReactNode } from 'react'

const DEFAULT_ROWS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'primary', length: 'long' },
  { id: 'secondary', length: 'medium' },
  { id: 'tertiary', length: 'xlong' },
  { id: 'quaternary', length: 'long' },
]

export function PendingSurface({
  children,
  className,
  delay = false,
  label = 'Loading page',
  onRetry,
  retryLabel = 'try again',
  delayedLabel = 'this is taking longer than usual',
}: {
  children?: ReactNode
  className?: string
  delay?: boolean
  label?: string
  onRetry?: () => void
  retryLabel?: string
  delayedLabel?: string
}) {
  const [delayed, setDelayed] = useState(false)
  useEffect(() => {
    if (delayed) return
    const timer = window.setTimeout(() => setDelayed(true), 8_000)
    return () => window.clearTimeout(timer)
  }, [delayed])
  return (
    <div
      aria-busy={delayed ? undefined : true}
      className={cn(
        'scope-pending-enter block min-h-full w-full',
        delay && 'scope-pending-delayed',
        className,
      )}
      data-slot="pending-surface"
    >
      {delayed && onRetry ? (
        <div className="flex min-h-[220px] flex-col items-center justify-center gap-3 px-6 py-10 text-center">
          <output className="block text-sm font-medium">{delayedLabel}</output>
          <p className="text-sm text-muted-foreground">you can keep waiting or try again</p>
          <Button
            onClick={() => {
              setDelayed(false)
              onRetry()
            }}
            size="sm"
            variant="secondary"
          >
            {retryLabel}
          </Button>
        </div>
      ) : (
        <>
          {delayed ? (
            <output className="block px-6 py-4 text-sm text-muted-foreground">{delayedLabel}. You can keep waiting.</output>
          ) : <output className="sr-only">{label}</output>}
          {children ?? <DefaultPageSkeleton />}
        </>
      )}
    </div>
  )
}

export function ApplicationPendingShell({
  children,
  contextLabel,
  label,
  repository,
}: {
  children?: ReactNode
  contextLabel?: string
  label: string
  repository?: { owner: string; repo: string }
}) {
  return (
    <AppShell
      header={() => (
        <ApplicationTopbar
          contextLabel={contextLabel}
          repository={repository}
        />
      )}
    >
      <PageRail className="min-h-full">
        <PendingSurface label={label}>{children}</PendingSurface>
      </PageRail>
    </AppShell>
  )
}

function DefaultPageSkeleton() {
  return (
    <div className="py-8 lg:py-10">
      <TextSkeleton length="medium" size="heading" />
      <TextSkeleton className="mt-3" length="long" />
      <div className="mt-8 divide-y divide-border border-y border-border">
        {DEFAULT_ROWS.map((row) => (
          <div className="py-5" key={row.id}>
            <TextSkeleton length={row.length} />
            <TextSkeleton className="mt-2" length="medium" size="meta" />
          </div>
        ))}
      </div>
    </div>
  )
}
