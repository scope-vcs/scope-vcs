import { PageHeader } from '@/components/page-header'
import { ApplicationPendingShell } from '@/components/pending-surface'
import {
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const PENDING_REPOSITORIES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'short' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
]

export function OwnerProfilePending({ owner }: { owner: string }) {
  return (
    <ApplicationPendingShell label={`Loading @${owner}`}>
      <div className="py-8 lg:py-10">
        <PageHeader title={`@${owner}`} />
        {/* Rows match RepoList as a visitor sees it: one line per repository. */}
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
