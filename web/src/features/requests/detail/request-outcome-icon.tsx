import { GitMerge, GitPullRequestClosed, type LucideProps } from 'lucide-react'
import type { RequestOutcome } from './request-labels'
import './request-outcome-icon.css'

const OUTCOME_ICONS = { merged: GitMerge, closed: GitPullRequestClosed } as const

export function RequestOutcomeIcon({ outcome, ...props }: Omit<LucideProps, 'ref'> & { outcome: RequestOutcome }) {
  const Icon = OUTCOME_ICONS[outcome]
  return <Icon aria-hidden="true" {...props} data-outcome={outcome} />
}
