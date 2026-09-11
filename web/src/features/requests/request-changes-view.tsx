import type {
  RequestRevisions,
  ReviewFileDiff,
} from '@/api/types'
import type { LoadRequestRevisionCommitInput } from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { GitCommit } from 'lucide-react'
import {
  RequestChangesWorkbench,
  type RequestChangesDiscussionReferences,
  type RequestChangesSearch,
} from './request-changes-workbench'
import type {
  LoadDiscussionsInput,
} from './request-discussion-api'
import type { RequestDiscussionPage } from './request-discussion-types'

type RequestChangesViewProps = {
  audience: 'private' | 'public'
  initialDiscussionReferences: RequestChangesDiscussionReferences
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
    signal?: AbortSignal,
  ) => Promise<ReviewFileDiff>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  onRetry: () => void
  retrying: boolean
  onSearchChange: (search: RequestChangesSearch) => void
  params: {
    owner: string
    repo: string
    request_id: string
  }
  repoId: string
  revisions: RequestRevisions | null
  scope: string | null
  search: RequestChangesSearch
}

export function RequestChangesView({
  audience,
  initialDiscussionReferences,
  loadDiff,
  loadDiscussions,
  onRetry,
  retrying,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
  scope,
}: RequestChangesViewProps) {
  if (!revisions) {
    return (
      <EmptyState
        description="the discussion is still available. Try loading this revision again."
        action={
          <div className="flex flex-wrap justify-center gap-3">
            <Button disabled={retrying} onClick={onRetry}>
              {retrying ? 'retrying changes…' : 'retry changes'}
            </Button>
            <Button asChild variant="secondary">
              <Link to="/$owner/$repo/requests/$requestId" params={{ owner: params.owner, repo: params.repo, requestId: params.request_id }}>
                back to discussion
              </Link>
            </Button>
            <output className="sr-only">{retrying ? 'loading request changes' : 'changes could not load'}</output>
          </div>
        }
        icon={<GitCommit />}
        title="changes couldn't load"
      />
    )
  }

  return (
    <RequestChangesWorkbench
      audience={audience}
      initialDiscussionReferences={initialDiscussionReferences}
      loadDiff={loadDiff}
      loadDiscussions={loadDiscussions}
      onSearchChange={onSearchChange}
      params={params}
      repoId={repoId}
      revisions={revisions}
      search={search}
      scope={scope}
    />
  )
}
