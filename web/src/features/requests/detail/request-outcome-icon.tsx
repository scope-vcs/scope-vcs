import type { RequestState } from '@/api/types.generated'
import { GitMerge, GitPullRequestClosed, type LucideProps } from 'lucide-react'
import './request-outcome-icon.css'

export type RequestOutcome = 'merged' | 'closed'

const OUTCOME_ICONS = { merged: GitMerge, closed: GitPullRequestClosed } as const

export function requestOutcome(state: RequestState): RequestOutcome | null {
  if (state === 'Merged') return 'merged'
  if (state === 'Closed') return 'closed'
  return null
}

export function RequestOutcomeIcon({ outcome, ...props }: Omit<LucideProps, 'ref'> & { outcome: RequestOutcome }) {
  const Icon = OUTCOME_ICONS[outcome]
  return <Icon aria-hidden="true" {...props} data-outcome={outcome} />
}
