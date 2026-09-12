import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'
import Markdown from 'react-markdown'
import { MarkdownLink } from './markdown-link'

export type MarkdownScale = 'compact' | 'document'

type MarkdownComponents = NonNullable<ComponentProps<typeof Markdown>['components']>

/** Spacing and type scale per rendering context; the element structure is shared. */
const SCALE_CLASSES = {
  compact: {
    a: 'decoration-border-strong hover:decoration-foreground',
    blockquote: 'my-3 border-border-strong pl-3',
    h1: 'mb-3 mt-5 text-xl',
    h2: 'mb-2 mt-5 text-lg',
    h3: 'mb-2 mt-4 text-base',
    hr: 'my-4',
    img: 'my-3',
    li: 'my-1',
    list: 'my-3',
    p: 'my-2',
    pre: 'my-3 rounded-md border border-border p-3 leading-5',
    table: 'my-3 border-y border-border',
    tableCell: 'py-2',
  },
  document: {
    a: 'decoration-[var(--platinum)]/70 hover:decoration-[var(--platinum-bright)]',
    blockquote: 'my-6 border-[var(--platinum)] pl-4',
    h1: 'mb-5 border-b border-border pb-5 text-3xl leading-tight tracking-[-0.03em] sm:text-[36px]',
    h2: 'mb-3 mt-9 border-b border-border pb-3 text-2xl tracking-[-0.025em]',
    h3: 'mb-2 mt-7 text-xl tracking-[-0.02em]',
    hr: 'my-8',
    img: 'my-4',
    li: 'my-1.5',
    list: 'my-5',
    p: 'my-4',
    pre: 'my-6 rounded-lg border border-border border-l-2 border-l-[var(--platinum)] p-4 leading-6 shadow-[var(--shadow-card)]',
    table: 'my-6 rounded-lg border border-border',
    tableCell: 'py-2.5',
  },
} as const

function buildMarkdownComponents(scale: MarkdownScale) {
  const classes = SCALE_CLASSES[scale]
  return {
    a: ({ children, className, href, ...props }: ComponentProps<'a'>) =>
      <MarkdownLink
        className={cn('font-medium text-foreground underline underline-offset-4', classes.a, className)}
        href={href}
        {...props}
      >
        {children}
      </MarkdownLink>,
    blockquote: ({ className, ...props }: ComponentProps<'blockquote'>) => (
      <blockquote className={cn('border-l-2 text-muted-foreground', classes.blockquote, className)} {...props} />
    ),
    code: ({ className, ...props }: ComponentProps<'code'>) => (
      <code
        className={cn('rounded bg-muted px-1.5 py-0.5 font-mono text-[0.88em] text-foreground', className)}
        {...props}
      />
    ),
    h1: ({ children, className, ...props }: ComponentProps<'h1'>) => (
      <h1 className={cn('font-semibold', classes.h1, className)} {...props}>{children}</h1>
    ),
    h2: ({ children, className, ...props }: ComponentProps<'h2'>) => (
      <h2 className={cn('font-semibold', classes.h2, className)} {...props}>{children}</h2>
    ),
    h3: ({ children, className, ...props }: ComponentProps<'h3'>) => (
      <h3 className={cn('font-semibold', classes.h3, className)} {...props}>{children}</h3>
    ),
    hr: ({ className, ...props }: ComponentProps<'hr'>) => (
      <hr className={cn('border-border', classes.hr, className)} {...props} />
    ),
    img: ({ alt }: ComponentProps<'img'>) => (
      <span className={cn('block border-l-2 border-border pl-3 text-sm italic text-muted-foreground', classes.img)}>
        Image omitted{alt ? `: ${alt}` : ''}
      </span>
    ),
    li: ({ className, ...props }: ComponentProps<'li'>) => (
      <li className={cn('pl-1', classes.li, className)} {...props} />
    ),
    ol: ({ className, ...props }: ComponentProps<'ol'>) => (
      <ol className={cn('list-decimal pl-6', classes.list, className)} {...props} />
    ),
    p: ({ className, ...props }: ComponentProps<'p'>) => (
      <p className={cn('text-pretty', classes.p, className)} {...props} />
    ),
    pre: ({ className, ...props }: ComponentProps<'pre'>) => (
      <pre
        className={cn(
          'overflow-x-auto bg-[var(--terminal-surface)] font-mono text-xs text-[var(--terminal-foreground)] [&_code]:bg-transparent [&_code]:p-0 [&_code]:text-inherit',
          classes.pre,
          className,
        )}
        {...props}
      />
    ),
    table: ({ className, ...props }: ComponentProps<'table'>) => (
      <div className={cn('overflow-x-auto', classes.table)}>
        <table className={cn('w-full border-collapse text-sm', className)} {...props} />
      </div>
    ),
    td: ({ className, ...props }: ComponentProps<'td'>) => (
      <td className={cn('border-b border-border px-3', classes.tableCell, className)} {...props} />
    ),
    th: ({ className, ...props }: ComponentProps<'th'>) => (
      <th
        className={cn('border-b border-border bg-muted px-3 text-left text-xs font-semibold', classes.tableCell, className)}
        {...props}
      />
    ),
    ul: ({ className, ...props }: ComponentProps<'ul'>) => (
      <ul className={cn('list-disc pl-6', classes.list, className)} {...props} />
    ),
  } satisfies MarkdownComponents
}

const MARKDOWN_COMPONENTS = {
  compact: buildMarkdownComponents('compact'),
  document: buildMarkdownComponents('document'),
}

/** The one set of markdown element overrides, at the spacing of the surface. */
export function markdownComponents(scale: MarkdownScale) {
  return MARKDOWN_COMPONENTS[scale]
}
