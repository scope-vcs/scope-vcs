import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
} from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'

const PENDING_DIFF_LINES: {
  highlighted?: boolean
  id: string
  length: LineSkeletonLength
}[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'short' },
  { id: 'third', length: 'long' },
  { highlighted: true, id: 'fourth', length: 'medium' },
  { highlighted: true, id: 'fifth', length: 'long' },
  { id: 'sixth', length: 'short' },
  { id: 'seventh', length: 'long' },
  { id: 'eighth', length: 'medium' },
  { id: 'ninth', length: 'long' },
]

/** The diff body while a file diff loads. Every diff view shares this one. */
export function DiffSkeleton() {
  return (
    <div className="py-3 font-mono">
      {PENDING_DIFF_LINES.map((line) => (
        <div
          className={cn(
            'grid min-h-7 grid-cols-[36px_minmax(0,1fr)] items-center gap-3 px-4',
            line.highlighted ? 'bg-success-soft/50' : undefined,
          )}
          key={line.id}
        >
          <TextSkeleton length="tiny" size="meta" />
          <LineSkeleton length={line.length} />
        </div>
      ))}
    </div>
  )
}

/**
 * ReviewFileDiffDrawer before its file is known: the same header row and body,
 * without importing the drawer and its renderers into a pending state.
 */
export function DiffDrawerSkeleton() {
  return (
    <div className="h-full min-h-[340px] bg-background">
      <div className="flex min-h-14 items-center gap-3 border-b border-border px-3 py-2.5">
        <BlockSkeleton className="size-4 shrink-0" />
        <div className="min-w-0 flex-1">
          <TextSkeleton length="medium" />
          <TextSkeleton length="tiny" size="meta" />
        </div>
      </div>
      <DiffSkeleton />
    </div>
  )
}
