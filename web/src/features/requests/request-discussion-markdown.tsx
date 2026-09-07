import { compactMarkdownComponents } from '@/components/compact-markdown-components'
import { SafeMarkdown } from '@/components/safe-markdown'
import { cn } from '@/lib/utils'

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
      components={compactMarkdownComponents}
    >
      {source}
    </SafeMarkdown>
  )
}
