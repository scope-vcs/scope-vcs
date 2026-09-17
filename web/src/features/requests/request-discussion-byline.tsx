import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'
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
        'grid shrink-0 place-items-center rounded-full bg-muted font-medium uppercase text-muted-foreground',
        small ? 'size-5 text-[9px]' : 'size-8 text-[11px]',
      )}
    >
      {handle.slice(0, 2)}
    </div>
  )
}
