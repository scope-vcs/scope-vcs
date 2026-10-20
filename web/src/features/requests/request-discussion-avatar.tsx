import type { CSSProperties } from 'react'
import { cn } from '../../lib/utils'
import {
  discussionAuthorAvatarUrl,
  discussionAuthorName,
  discussionAuthorSourceUrl,
  type DiscussionAuthor,
} from './request-discussion-author'

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

export function RequestDiscussionAuthorSource({ author }: { author: DiscussionAuthor }) {
  const sourceUrl = discussionAuthorSourceUrl(author)
  if (!sourceUrl) return null
  return (
    <a
      className="whitespace-nowrap text-[13px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-ring"
      href={sourceUrl}
      rel="noreferrer"
      target="_blank"
    >
      via GitHub
    </a>
  )
}

export function RequestDiscussionAuthorAvatar({
  author,
  small = false,
}: {
  author: DiscussionAuthor
  small?: boolean
}) {
  return (
    <RequestDiscussionActorAvatar
      handle={discussionAuthorName(author)}
      imageUrl={discussionAuthorAvatarUrl(author)}
      small={small}
    />
  )
}

export function RequestDiscussionActorAvatar({
  handle,
  imageUrl = null,
  small = false,
}: {
  handle: string
  imageUrl?: string | null
  small?: boolean
}) {
  if (imageUrl) {
    return (
      <img
        alt=""
        aria-hidden="true"
        className={cn('shrink-0 rounded-full bg-muted object-cover', small ? 'size-5' : 'size-8')}
        loading="lazy"
        src={imageUrl}
      />
    )
  }
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
