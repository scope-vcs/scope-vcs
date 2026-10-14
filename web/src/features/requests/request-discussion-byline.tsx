import { cn } from '@/lib/utils'
import type { CSSProperties, ReactNode } from 'react'
import { actorHandle, type RecordedActor } from './request-actor'
import { RelativeTimestamp } from '@/components/timestamp'

const ACTOR_HUE_PAIRS = [
  [255, 295],
  [215, 255],
  [175, 215],
  [140, 175],
  [20, 350],
  [305, 335],
  [45, 20],
  [275, 225],
  [335, 15],
  [295, 330],
  [65, 35],
  [190, 150],
] as const

export function RequestDiscussionByline({
  author,
  children,
  createdAtUnix,
  small = false,
}: {
  author: RecordedActor
  children?: ReactNode
  createdAtUnix: number
  small?: boolean
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
      <span
        className={cn('truncate font-semibold', small ? 'text-[13px]' : 'text-sm')}
      >
        {actorHandle(author)}
      </span>
      <RelativeTimestamp
        className="whitespace-nowrap text-[13px] text-muted-foreground"
        value={createdAtUnix}
      />
      {children}
    </div>
  )
}

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
        small ? 'size-5 text-[9px]' : 'size-8 text-xs',
      )}
      style={{ '--actor-hue': actorHues(handle).hue } as CSSProperties}
    >
      {handle.slice(0, 2)}
    </div>
  )
}

export function actorHues(handle: string) {
  let hash = 0
  for (const char of handle) hash = (hash * 31 + char.charCodeAt(0)) >>> 0
  const [hue, accent] = ACTOR_HUE_PAIRS[hash % ACTOR_HUE_PAIRS.length]
  return { hue, accent, angle: (hash >>> 5) % 360 }
}
