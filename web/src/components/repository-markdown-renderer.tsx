import { cn } from '@/lib/utils'
import rehypeSlug from 'rehype-slug'
import { markdownComponents } from './markdown-components'
import { SafeMarkdown } from './safe-markdown'
import {
  REPOSITORY_MARKDOWN_HEADING_PREFIX,
  resolveRepositoryMarkdownUrl,
  type RepositoryMarkdownContext,
} from './repository-markdown'

export function RepositoryMarkdownRenderer({
  className,
  repository,
  source,
}: {
  className?: string
  repository: RepositoryMarkdownContext
  source: string
}) {
  return (
    <article
      className={cn(
        'mx-auto w-full max-w-[760px] px-6 py-8 text-[15px] leading-7 text-foreground sm:px-10 sm:py-10',
        className,
      )}
    >
      <SafeMarkdown
        components={markdownComponents('document')}
        rehypePlugins={[[rehypeSlug, { prefix: REPOSITORY_MARKDOWN_HEADING_PREFIX }]]}
        urlTransform={(url) => resolveRepositoryMarkdownUrl(url, repository)}
      >
        {source}
      </SafeMarkdown>
    </article>
  )
}
