import { cn } from '@/lib/utils'
import { Clock3, MessageSquarePlus, Reply, RotateCcw } from 'lucide-react'
import { useState } from 'react'
import { RequestAttachmentEditor } from './request-attachment-editor'

/**
 * Sits at the end of the list, where a new discussion lands. Collapsed it is a
 * single line; activating it opens the full composer.
 */
export function RequestDiscussionComposer({
  onSubmit,
}: {
  onSubmit: (body: string) => Promise<boolean>
}) {
  const [open, setOpen] = useState(false)

  if (!open) {
    return (
      <button
        className={cn(
          'flex w-full items-center gap-2 rounded-md border border-input bg-background',
          'px-3 py-2 text-left text-sm text-muted-foreground',
          'hover:border-ring hover:text-foreground',
          'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none',
        )}
        onClick={() => setOpen(true)}
        type="button"
      >
        <MessageSquarePlus className="size-3.5 shrink-0" />
        Start a discussion about this request…
      </button>
    )
  }

  return (
    <RequestAttachmentEditor
      autoFocus
      label="Start a new discussion"
      onCancel={() => setOpen(false)}
      onSubmit={async (body) => {
        const posted = await onSubmit(body)
        if (posted) setOpen(false)
        return posted
      }}
      placeholder="Start a focused discussion about this request…"
      submitIcon={<MessageSquarePlus className="size-3.5" />}
      submitLabel="Start discussion"
      target="discussion"
    />
  )
}

export function RequestReplyComposer({
  discussionId,
  onCancel,
  onCancelQuote,
  onSubmit,
  quote,
  reopen,
  waitAfterReply,
}: {
  discussionId: string
  onCancel: () => void
  onCancelQuote: () => void
  onSubmit: (body: string) => Promise<boolean>
  quote: { author: string; body: string } | null
  reopen: boolean
  waitAfterReply?: (body: string) => Promise<boolean>
}) {
  return (
    <RequestAttachmentEditor
      autoFocus
      label={reopen ? 'Reopen and reply' : 'Reply'}
      onCancel={onCancel}
      onSubmit={onSubmit}
      placeholder={
        reopen
          ? 'Explain why this discussion needs to continue…'
          : 'Add a reply…'
      }
      quote={quote}
      onCancelQuote={onCancelQuote}
      secondarySubmit={waitAfterReply ? {
        icon: <Clock3 className="size-3.5" />,
        label: 'Reply & wait',
        onSubmit: waitAfterReply,
      } : undefined}
      submitIcon={
        reopen ? (
          <RotateCcw className="size-3.5" />
        ) : (
          <Reply className="size-3.5" />
        )
      }
      submitLabel={reopen ? 'Reopen and reply' : 'Reply'}
      target={`reply:${discussionId}`}
    />
  )
}
