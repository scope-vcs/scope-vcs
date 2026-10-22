import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'
import { discussionAuthorName, type DiscussionAuthor } from './request-discussion-author'
import { RequestDiscussionAuthorSource } from './request-discussion-avatar'
import { RelativeTimestamp } from '@/components/timestamp'

export function RequestDiscussionByline({
  author,
  children,
  createdAtUnix,
  small = false,
}: {
  author: DiscussionAuthor
  children?: ReactNode
  createdAtUnix: number
  small?: boolean
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
      <span
        className={cn('truncate font-semibold', small ? 'text-[13px]' : 'text-sm')}
      >
        {discussionAuthorName(author)}
      </span>
      <RelativeTimestamp
        className="whitespace-nowrap text-[13px] text-muted-foreground"
        value={createdAtUnix}
      />
      <RequestDiscussionAuthorSource author={author} />
      {children}
    </div>
  )
}
