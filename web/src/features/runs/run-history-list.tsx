import type { RepoParams } from '@/api/types'
import type { RepositoryRunHistoryPageResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/empty-state'
import { LoaderCircle, TerminalSquare } from 'lucide-react'
import type { ReactNode } from 'react'
import { RunRow } from './run-row'

/** A repository's Runs page before any of its workflows ran. */
export function NoRunsEmptyState({ selectedWorkflowName }: { selectedWorkflowName?: string }) {
  return (
    <EmptyState
      description="Push to main with a matching trigger, or run a workflow manually from the CLI."
      icon={<TerminalSquare />}
      title={selectedWorkflowName ? `No ${selectedWorkflowName} runs yet` : 'No runs yet'}
    />
  )
}

/** `empty` replaces the usual state for a repository without runs. */
export function RunHistoryList({
  empty,
  loadMore,
  loadingMore,
  params,
  runs,
  selectedWorkflowName,
  showLoadMore,
  totalRunCount,
}: {
  empty?: ReactNode
  loadMore: () => void
  loadingMore: boolean
  params: RepoParams
  runs: RepositoryRunHistoryPageResponse['runs']
  selectedWorkflowName?: string
  showLoadMore: boolean
  totalRunCount: number
}) {
  if (totalRunCount === 0) {
    return empty ?? <NoRunsEmptyState selectedWorkflowName={selectedWorkflowName} />
  }

  return (
    <div>
      {runs.length === 0 ? (
        <EmptyState
          description="Older runs may still match. Load more history, or try a different filter."
          icon={<TerminalSquare />}
          title="No runs match this filter"
        />
      ) : (
        <div className="divide-y divide-border">
          {runs.map((run) => (
            <RunRow key={run.id} params={params} run={run} />
          ))}
        </div>
      )}
      {showLoadMore || runs.length > 0 ? (
        <div className="flex items-center justify-center gap-3 pt-5">
          {showLoadMore ? (
            <Button aria-busy={loadingMore} disabled={loadingMore} onClick={loadMore} variant="secondary">
              {loadingMore ? <LoaderCircle className="animate-spin" /> : null}
              Load older runs
            </Button>
          ) : null}
          <span className="text-xs text-muted-foreground">
            Showing {runs.length}
          </span>
        </div>
      ) : null}
    </div>
  )
}
