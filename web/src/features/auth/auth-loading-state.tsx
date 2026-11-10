import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton } from '@/components/ui/block-skeleton'

export function AuthLoadingState({ label }: { label: string }) {
  return (
    <PendingSurface
      className="min-h-[220px] w-full"
      label={label}
      onRetry={() => window.location.reload()}
    >
      <div className="space-y-3">
        <BlockSkeleton className="h-11 w-full" />
        <BlockSkeleton className="h-11 w-full" />
        <BlockSkeleton className="h-11 w-full" />
      </div>
    </PendingSurface>
  )
}
