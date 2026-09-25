import { cn } from '@/lib/utils'
import type { ComponentProps } from 'react'

// Each size takes the line height of the text it stands in for and draws the
// bar inside it, so a skeleton row is as tall as the loaded row.
const TEXT_SIZE = {
  body: 'h-5 py-0.5',
  // Page headings are 26 to 28px on phones and 30 to 32px from sm up.
  heading: 'h-[30px] py-1 sm:h-9',
  meta: 'h-4 py-0.5',
  title: 'h-6 py-0.5',
} as const

const TEXT_WIDTH = {
  long: '32ch',
  medium: '20ch',
  short: '12ch',
  tiny: '4ch',
  xlong: '48ch',
} as const

export type TextSkeletonLength = keyof typeof TEXT_WIDTH

export function TextSkeleton({
  className,
  length = 'medium',
  size = 'body',
  ...props
}: Omit<ComponentProps<'span'>, 'style'> & {
  length?: TextSkeletonLength
  size?: keyof typeof TEXT_SIZE
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'scope-skeleton block max-w-full rounded-md bg-muted bg-clip-content',
        TEXT_SIZE[size],
        className,
      )}
      data-skeleton-kind="text"
      data-slot="skeleton"
      style={{ width: TEXT_WIDTH[length] }}
      {...props}
    />
  )
}
