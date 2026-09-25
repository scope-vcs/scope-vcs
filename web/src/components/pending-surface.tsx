import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageRail } from '@/components/page-header'
import {
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useRouter } from '@tanstack/react-router'
import { createContext, use, useCallback, useEffect, useState, type ReactNode } from 'react'

const DEFAULT_ROWS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'primary', length: 'long' },
  { id: 'secondary', length: 'medium' },
  { id: 'tertiary', length: 'xlong' },
  { id: 'quaternary', length: 'long' },
]

// A surface inside another surface leaves the announcement and the slow note
// to the outer one, so a page announces its loading once.
const NestedSurface = createContext(false)

const SLOW_AFTER_MS = 8_000

// Router fallbacks for one navigation share its start, so when a parent's
// fallback hands off to a child's, the child does not restart the eight seconds.
let navigationPending: { key: string; since: number } | null = null

function navigationSince(router: NonNullable<ReturnType<typeof useRouter>>) {
  const { href, state } = router.state.location
  const key = state.__TSR_key ?? href
  if (navigationPending?.key !== key) navigationPending = { key, since: Date.now() }
  return navigationPending.since
}

/**
 * One policy for every loading state. A surface the router shows in place of a
 * page appears at once, because the router has already waited before showing
 * it. A surface a loaded page draws for its own data waits the same 150ms, so
 * fast loads never flash. After eight seconds a note appears over the
 * surface's top corner without moving anything.
 */
export function PendingSurface({
  children,
  className,
  label = 'Loading page',
  onRetry,
}: {
  children?: ReactNode
  className?: string
  label?: string
  /** Retries this surface's own load. Router fallbacks retry the navigation. */
  onRetry?: () => void
}) {
  const nested = use(NestedSurface)
  const router = useRouter({ warn: false })
  const [routeFallback] = useState(() => router?.state.status === 'pending')
  const [since] = useState(() => routeFallback && router ? navigationSince(router) : Date.now())
  const [slow, restartSlow] = useSlow(!nested, since)
  const retryLoad = onRetry ?? (routeFallback && router ? () => void router.invalidate() : undefined)
  // A retry starts a fresh attempt, which gets its own eight seconds.
  const retry = retryLoad && (() => {
    restartSlow()
    retryLoad()
  })
  return (
    <NestedSurface value>
      {nested ? null : (
        <output className="sr-only">{slow ? `${label}. Still loading.` : label}</output>
      )}
      <div
        aria-busy="true"
        className={cn(
          'scope-pending-enter relative block min-h-full w-full',
          !routeFallback && 'scope-pending-delayed',
          className,
        )}
        data-slot="pending-surface"
      >
        {slow ? <SlowNote label={label} onRetry={retry} /> : null}
        {children ?? <DefaultPageSkeleton />}
      </div>
    </NestedSurface>
  )
}

function useSlow(enabled: boolean, since: number) {
  const [startedAt, setStartedAt] = useState(since)
  const [slow, setSlow] = useState(false)
  useEffect(() => {
    if (!enabled || slow) return
    const timer = window.setTimeout(
      () => setSlow(true),
      Math.max(0, SLOW_AFTER_MS - (Date.now() - startedAt)),
    )
    return () => window.clearTimeout(timer)
  }, [enabled, slow, startedAt])
  const restart = useCallback(() => {
    setSlow(false)
    setStartedAt(Date.now())
  }, [])
  return [slow, restart] as const
}

function SlowNote({ label, onRetry }: { label: string; onRetry?: () => void }) {
  return (
    <div className="absolute right-3 top-3 z-10 flex items-center gap-2 rounded-md border border-border bg-background/95 px-2.5 py-1 text-xs text-muted-foreground shadow-sm">
      <span>Still loading</span>
      {onRetry ? (
        <>
          <span aria-hidden="true">·</span>
          <button
            aria-label={`Retry ${label.charAt(0).toLowerCase()}${label.slice(1)}`}
            className="rounded font-medium text-foreground underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-ring"
            onClick={onRetry}
            type="button"
          >
            Retry
          </button>
        </>
      ) : null}
    </div>
  )
}

export function ApplicationPendingShell({
  actions,
  children,
  contextLabel,
  label,
}: {
  /** The loaded page's topbar controls, when they are known before its data. */
  actions?: ReactNode
  children?: ReactNode
  contextLabel?: string
  label: string
}) {
  return (
    <AppShell
      header={() => (
        <ApplicationTopbar contextLabel={contextLabel}>{actions}</ApplicationTopbar>
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
