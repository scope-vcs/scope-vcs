import { PageHeader } from '@/components/page-header'
import { ApplicationPendingShell } from '@/components/pending-surface'
import { TextSkeleton, type TextSkeletonLength } from '@/components/ui/text-skeleton'

const REPOSITORY_LENGTHS: TextSkeletonLength[] = ['medium', 'short', 'long', 'medium']
const PENDING_REPOSITORIES = Array.from({ length: 12 }, (_, row) => ({
  id: `repository-${row}`,
  length: REPOSITORY_LENGTHS[row % REPOSITORY_LENGTHS.length],
}))

export function OwnerProfilePending({ owner }: { owner: string }) {
  return (
    <ApplicationPendingShell label={`Loading @${owner}`}>
      <div className="py-8 lg:py-10">
        <PageHeader title={`@${owner}`} />
        <div className="mt-6 divide-y divide-border">
          {PENDING_REPOSITORIES.map((repository) => (
            <div className="py-3" key={repository.id}>
              <TextSkeleton length={repository.length} size="title" />
            </div>
          ))}
        </div>
      </div>
    </ApplicationPendingShell>
  )
}
