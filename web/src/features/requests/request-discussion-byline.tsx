import { cn } from '@/lib/utils'
import type { CSSProperties, ReactNode } from 'react'
import type { RequestActorSummary } from './request-discussion-types'
import { RelativeTimestamp } from '@/components/timestamp'

/**
 * Author, time, and state for one discussion or reply. Threads and replies
 * share it so the two never drift apart again.
 */
export function RequestDiscussionByline({
  author,
  children,
  createdAtUnix,
  small = false,
}: {
  author: RequestActorSummary
  children?: ReactNode
  createdAtUnix: number
  small?: boolean
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
      <span
        className={cn('truncate font-semibold', small ? 'text-[13px]' : 'text-sm')}
      >
        {author.handle}
      </span>
      <RelativeTimestamp
        className="whitespace-nowrap text-[13px] text-muted-foreground"
        value={createdAtUnix}
      />
      {children}
    </div>
  )
}

/**
 * Initials on a colour picked from the handle, so the same person reads the
 * same everywhere and different people read apart at a glance.
 */
export function RequestDiscussionActorAvatar({
  handle,
  small = false,
}: {
  handle: string
  small?: boolean
}) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        'grid shrink-0 place-items-center rounded-full font-medium uppercase',
        'bg-[oklch(0.9_0.045_var(--actor-hue))] text-[oklch(0.42_0.08_var(--actor-hue))]',
        'dark:bg-[oklch(0.34_0.05_var(--actor-hue))] dark:text-[oklch(0.86_0.06_var(--actor-hue))]',
        small ? 'size-5 text-[9px]' : 'size-8 text-[11px]',
      )}
      style={{ '--actor-hue': actorHue(handle) } as CSSProperties}
    >
      {handle.slice(0, 2)}
    </div>
  )
}

/** One of twelve hues 30 degrees apart, stable for a handle. */
function actorHue(handle: string) {
  let hash = 0
  for (const char of handle) hash = (hash * 31 + char.charCodeAt(0)) >>> 0
  return (hash % 12) * 30
}
