import { PageHeader } from '@/components/page-header'
import { useAuth } from '@clerk/tanstack-react-start'
import { ApplicationPendingShell } from '@/components/pending-surface'
import {
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { OwnerProfileTopbarActions } from './owner-profile-topbar-actions'

const REPOSITORY_LENGTHS: TextSkeletonLength[] = ['medium', 'short', 'long', 'medium']
// Enough rows to fill a first screen, so dividers line up with any list length.
const PENDING_REPOSITORIES = Array.from({ length: 12 }, (_, row) => ({
  id: `repository-${row}`,
  length: REPOSITORY_LENGTHS[row % REPOSITORY_LENGTHS.length],
}))

export function OwnerProfilePending({ owner }: { owner: string }) {
  const { isSignedIn } = useAuth()
  return (
    <ApplicationPendingShell
      actions={<OwnerProfileTopbarActions handle={owner} signedIn={Boolean(isSignedIn)} />}
      label={`Loading @${owner}`}
    >
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
