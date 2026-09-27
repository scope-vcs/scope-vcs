import { cn } from '../../lib/utils'
import { ChevronDown } from 'lucide-react'
import {
  cacheSetupLabel,
  cacheSizeLabel,
  cacheStateClass,
  cacheStateDetail,
  cacheStateLabel,
  cacheTimingLabel,
  cachesNeedAttention,
  pinnedImageLabel,
} from './run-attempt-environment-model'
import type {
  RepositoryRunAttemptResponse,
  RepositoryRunCacheResponse,
} from '@/api/types.generated'

const ENVIRONMENT_ROW_CLASS =
  'grid grid-cols-[7rem_6rem_minmax(0,1fr)] items-baseline gap-3 py-1.5 sm:grid-cols-[9rem_7rem_6rem_minmax(0,1fr)]'

/** The collapsed Environment control. A dot flags a cold or unreported cache. */
export function RunEnvironmentToggle({
  caches,
  expanded,
  onToggle,
  panelId,
}: {
  caches: readonly RepositoryRunCacheResponse[]
  expanded: boolean
  onToggle: () => void
  panelId: string
}) {
  const attention = cachesNeedAttention(caches)
  return (
    <button
      aria-controls={panelId}
      aria-expanded={expanded}
      className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring aria-expanded:bg-muted aria-expanded:text-foreground"
      onClick={onToggle}
      title={attention ? 'A cache was cold or not reported' : undefined}
      type="button"
    >
      {attention ? <span aria-hidden="true" className="size-1.5 rounded-full bg-warning" /> : null}
      Environment
      <ChevronDown
        aria-hidden="true"
        className={cn('size-3.5 transition-transform', expanded && 'rotate-180')}
      />
    </button>
  )
}

/** Caches and image for one attempt: what each cache restored, its size and
 * how long it took. */
export function RunEnvironmentPanel({
  caches,
  cacheSetup,
  id,
  pinnedContainerImage,
}: {
  caches: readonly RepositoryRunCacheResponse[]
  cacheSetup: RepositoryRunAttemptResponse['cache_setup']
  id: string
  pinnedContainerImage: string | null
}) {
  const setup = cacheSetupLabel(cacheSetup)
  return (
    <section
      aria-label="Execution environment"
      className="divide-y divide-border/70 border-b border-border px-4 py-1 text-xs"
      id={id}
    >
      {caches.length === 0 ? (
        <p className="py-1.5 text-muted-foreground">No caches declared</p>
      ) : caches.map((cache) => (
        <div className={ENVIRONMENT_ROW_CLASS} key={cache.name}>
          <span className="truncate font-mono">{cache.name}</span>
          <span
            className={cn('font-medium', cacheStateClass(cache))}
            title={cacheStateDetail(cache) ?? undefined}
          >
            {cacheStateLabel(cache)}
          </span>
          <span className="hidden tabular-nums text-muted-foreground sm:block">
            {cacheSizeLabel(cache)}
          </span>
          <span className="tabular-nums text-muted-foreground">
            {cacheTimingLabel(cache)}
          </span>
        </div>
      ))}
      <div className={ENVIRONMENT_ROW_CLASS}>
        <span className="font-mono">image</span>
        <code
          className="col-span-2 truncate text-muted-foreground sm:col-span-3"
          title={pinnedContainerImage ?? undefined}
        >
          {pinnedImageLabel(pinnedContainerImage)}
        </code>
      </div>
      {setup ? <p className="py-1.5 text-muted-foreground">{setup}</p> : null}
    </section>
  )
}
