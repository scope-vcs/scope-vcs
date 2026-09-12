import { markdownComponents } from '@/components/markdown-components'
import { SafeMarkdown } from '@/components/safe-markdown'
import { cn } from '@/lib/utils'
import type { ComponentProps, ReactNode } from 'react'
import { RequestAttachmentMedia } from './request-attachment-media'
import { requestAttachmentIdFromUrl } from './request-attachment-reference'

const compactMarkdownComponents = markdownComponents('compact')

const requestMarkdownComponents = {
  ...compactMarkdownComponents,
  a: ({ children, href, ...props }: ComponentProps<'a'>) => {
    const attachmentId = requestAttachmentIdFromUrl(href)
    return attachmentId
      ? <RequestAttachmentMedia attachmentId={attachmentId} label={textOf(children)} />
      : compactMarkdownComponents.a({ children, href, ...props })
  },
  img: ({ alt, src }: ComponentProps<'img'>) => {
    const attachmentId = requestAttachmentIdFromUrl(src)
    return attachmentId ? (
      <RequestAttachmentMedia attachmentId={attachmentId} label={alt ?? ''} />
    ) : (
      <span className="my-3 block border-l-2 border-border pl-3 text-sm italic text-muted-foreground">
        Image omitted{alt ? `: ${alt}` : ''}
      </span>
    )
  },
  p: ({ className, ...props }: ComponentProps<'p'>) => (
    <div className={cn('my-2 text-pretty [&>:first-child]:mt-0 [&>:last-child]:mb-0', className)} {...props} />
  ),
}

export function RequestDiscussionMarkdown({
  className,
  source,
}: {
  className?: string
  source: string
}) {
  return (
    <SafeMarkdown
      className={cn(
        'min-w-0 break-words text-base leading-[26px] [&>:first-child]:mt-0 [&>:last-child]:mb-0',
        className,
      )}
      components={requestMarkdownComponents}
    >
      {source}
    </SafeMarkdown>
  )
}

function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  return ''
}
