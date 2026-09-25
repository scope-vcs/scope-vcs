import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const PENDING_FILES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'long' },
  { id: 'third', length: 'short' },
  { id: 'fourth', length: 'long' },
  { id: 'fifth', length: 'medium' },
  { id: 'sixth', length: 'long' },
]

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

// Mirrors RepositoryFileNavigator: find-file search, then the file tree with
// its column label and rows.
export function FileNavigatorSkeleton() {
  return (
    <div>
      <div className="mb-2 px-1">
        <BlockSkeleton className="h-8 w-full" />
      </div>
      <div className="hidden px-3 pb-1.5 pt-1 text-[11px] font-medium text-muted-foreground sm:block">
        path
      </div>
      <ul className="space-y-0.5">
        {PENDING_FILES.map((file) => (
          <li
            className="grid min-h-9 grid-cols-[minmax(0,1fr)_18px] items-center gap-2 border border-transparent px-3 py-1.5"
            key={file.id}
          >
            <div className="flex min-w-0 items-center gap-2">
              <BlockSkeleton className="size-4 shrink-0" />
              <TextSkeleton length={file.length} size="meta" />
            </div>
            <BlockSkeleton className="size-3.5 rounded-full" />
          </li>
        ))}
      </ul>
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
