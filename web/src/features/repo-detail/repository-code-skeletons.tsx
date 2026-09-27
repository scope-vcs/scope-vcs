import { FileSystemTreeSkeleton } from '@/components/file-system-tree'
import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
} from '@/components/ui/skeleton'

const PENDING_SOURCE_LINES: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
  { id: 'fifth', length: 'short' },
  { id: 'sixth', length: 'long' },
  { id: 'seventh', length: 'medium' },
  { id: 'eighth', length: 'long' },
  { id: 'ninth', length: 'short' },
]

// Mirrors RepositoryFileNavigator: find-file search, then the file tree.
export function FileNavigatorSkeleton() {
  return (
    <div>
      <div className="mb-2 px-1">
        <BlockSkeleton className="h-8 w-full" />
      </div>
      <FileSystemTreeSkeleton metaColumnLabel={null} />
    </div>
  )
}

export function SourceCodeSkeleton() {
  return (
    <div className="space-y-3 p-5 sm:p-7">
      {PENDING_SOURCE_LINES.map((line) => (
        <div className="flex items-center gap-4" key={line.id}>
          <TextSkeleton length="tiny" size="meta" />
          <LineSkeleton length={line.length} />
        </div>
      ))}
    </div>
  )
}

/** The tab strip before the landing file's tab opens. */
export function SourceTabStripSkeleton() {
  return (
    <div className="flex min-h-10 items-center border-b border-border px-3">
      <TextSkeleton length="short" size="meta" />
    </div>
  )
}
