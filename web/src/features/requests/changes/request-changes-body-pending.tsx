import { PendingSurface } from '@/components/pending-surface'
import { TextSkeleton, type TextSkeletonLength } from '@/components/ui/text-skeleton'
import { DiffSkeleton } from '@/features/review/diff-skeleton'

const PENDING_CHANGES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
]

export function RequestChangesBodyPending() {
  return (
    <PendingSurface label="Loading request changes">
      <section className="grid border-t border-border lg:grid-cols-[minmax(220px,0.42fr)_minmax(0,1.58fr)]">
        <div className="divide-y divide-border border-b border-border lg:border-b-0 lg:border-r">
          {PENDING_CHANGES.map((change) => (
            <div className="px-5 py-4" key={change.id}>
              <TextSkeleton length={change.length} />
              <TextSkeleton className="mt-2" length="short" size="meta" />
            </div>
          ))}
        </div>
        <div className="min-h-[340px] p-5 lg:p-6">
          <TextSkeleton length="long" />
          <TextSkeleton className="mt-2" length="medium" size="meta" />
          <div className="mt-4">
            <DiffSkeleton />
          </div>
        </div>
      </section>
    </PendingSurface>
  )
}
