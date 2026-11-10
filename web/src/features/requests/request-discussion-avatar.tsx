import type { CSSProperties } from 'react'
import { cn } from '../../lib/utils'
import {
  discussionAuthorAvatarUrl,
  discussionAuthorName,
  discussionAuthorSourceLabel,
  discussionAuthorSourceUrl,
  type DiscussionAuthor,
} from './request-discussion-author'

const ACTOR_HUES = [255, 215, 175, 140, 20, 305, 45, 275, 335, 295, 65, 190]

export function RequestDiscussionAuthorSource({ author }: { author: DiscussionAuthor }) {
  const sourceUrl = discussionAuthorSourceUrl(author)
  if (!sourceUrl) return null
  return (
    <a
      aria-label={discussionAuthorSourceLabel(author) ?? undefined}
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
  const handle = discussionAuthorName(author)
  const imageUrl = discussionAuthorAvatarUrl(author)
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
      style={{ '--actor-hue': actorHue(handle) } as CSSProperties}
    >
      {handle.slice(0, 2)}
    </div>
  )
}

function actorHue(handle: string) {
  let hash = 0
  for (const char of handle) hash = (hash * 31 + char.charCodeAt(0)) >>> 0
  return ACTOR_HUES[hash % ACTOR_HUES.length]
}
