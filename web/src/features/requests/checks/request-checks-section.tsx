import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { requestChecksSummary, requestPullRequestView, requestReviewBotRows } from './request-check-rows'
import { RequestCheckTrack } from './request-check-track'
import { RequestReviewBots } from './request-review-bots'
import {
  requestCheckEvaluationNote,
  requestChecksWorkflowWarning,
} from '../detail/request-labels'
import { CHECKS_SECTION_CLASS } from './request-checks-pending'
import './request-checks.css'

export function RequestChecksSection({
  checks,
  error,
  params,
}: {
  checks: RequestChecksResponse
  error: string | null
  params: RepoParams
}) {
  const pullRequest = requestPullRequestView(checks.github_pull_request)
  const reviewBots = requestReviewBotRows(checks.review_bots)
  if (checks.state === 'no-checks' && !pullRequest && !reviewBots.length) return null

  const note = requestCheckEvaluationNote(checks)
  const warning = requestChecksWorkflowWarning(checks)
  const summary = requestChecksSummary(checks)
  const pullRequestNotice = pullRequest && (pullRequest.outOfStep || !reviewBots.length) ? pullRequest : null

  return (
    <section
      aria-label="CI"
      className={CHECKS_SECTION_CLASS}
    >
      <div className="request-checks-line pt-2.5 text-[13px] leading-5">
        <RequestCheckTrack params={params} summary={summary} />
        <RequestReviewBots bots={reviewBots} pullRequest={pullRequest} />
      </div>
      {error ? (
        <p className="mt-2 text-[13px] text-danger-strong" role="alert">
          {error}
        </p>
      ) : null}
      {note ? <p className="mt-2 text-[13px] text-muted-foreground">{note}</p> : null}
      {warning ? (
        <p className="mt-2 text-[13px] text-warning-strong" role="note">
          {warning}
        </p>
      ) : null}
      {summary.startError ? (
        <p
          className={cn(
            'mt-1 break-words text-[13px]',
            summary.startError.failed ? 'text-danger-strong' : 'text-muted-foreground',
          )}
        >
          {summary.startError.text}
        </p>
      ) : null}
      {pullRequestNotice ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {pullRequestNotice.url ? (
            <a className="hover:text-foreground hover:underline underline-offset-4" href={pullRequestNotice.url} rel="noreferrer" target="_blank">
              {pullRequestNotice.text}
            </a>
          ) : pullRequestNotice.text}
        </p>
      ) : null}
      {pullRequestNotice?.note ? (
        <p className={cn('mt-1 break-words text-xs', pullRequestNotice.note.failed ? 'text-danger-strong' : 'text-muted-foreground')}>
          {pullRequestNotice.note.text}
        </p>
      ) : null}
    </section>
  )
}
