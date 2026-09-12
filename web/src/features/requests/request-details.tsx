import type { RequestParams } from '@/api/types'
import type { RequestRatingResponse, RequestRatingsResponse, RequestSummaryResponse } from '@/api/types.generated'
import { shortOid } from '@/lib/short-oid'
import { GitCommitHorizontal } from 'lucide-react'
import { createContext, type ReactNode, use } from 'react'
import { RequestInvitees } from './request-invitees'
import { RequestRatingsSection } from './request-ratings-section'
import type { RateRequestInput } from '@/api/requests'
import {
  requestAudienceLabel,
  requestAuthorRoleLabel,
} from './request-labels'
import { AbsoluteTimestamp } from '@/components/timestamp'
import type { RequestActionController } from './use-request-actions'

type RequestDetailsProps = {
  actions: RequestActionController
  onRate: (input: RateRequestInput) => Promise<RequestRatingResponse>
  params: RequestParams
  ratings: RequestRatingsResponse
  request: RequestSummaryResponse
}

const RequestDetailsContext = createContext<RequestDetailsProps | null>(null)

export function RequestDetailsProvider({ children, value }: { children: ReactNode; value: RequestDetailsProps }) {
  return <RequestDetailsContext value={value}>{children}</RequestDetailsContext>
}

export function RequestDetails() {
  const context = use(RequestDetailsContext)
  if (!context) throw new Error('Request details context is unavailable')
  const { actions, onRate, params, ratings, request } = context
  return (
    <section aria-label="Request details" className="min-w-0 border-t border-border px-5 py-6 sm:px-6 lg:px-8">
      <div className="grid min-w-0 gap-x-12 gap-y-8 sm:grid-cols-2">
        <DetailsSection title="lifecycle">
          <DetailsValue label="Author" value={requestAuthorRoleLabel(request)} />
          <DetailsValue label="Audience" value={requestAudienceLabel(request)} />
          <DetailsValue
            label="Submitted"
            value={<AbsoluteTimestamp value={request.submitted_at_unix} />}
          />
          {request.closed_at_unix !== null && (
            <DetailsValue
              label="Closed"
              value={<AbsoluteTimestamp value={request.closed_at_unix} />}
            />
          )}
          {request.merged_at_unix !== null && (
            <DetailsValue
              label="Merged"
              value={<AbsoluteTimestamp value={request.merged_at_unix} />}
            />
          )}
        </DetailsSection>

        <RequestInvitees actions={actions} request={request} />

        <RequestRatingsSection initial={ratings} onRate={onRate} params={params} />

        <DetailsSection icon={<GitCommitHorizontal />} title="git state">
          <DetailsValue label="Base" value={shortOid(request.base_main_oid)} />
          <DetailsValue label="Head" value={shortOid(request.head_oid)} />
          <pre className="mt-1 min-w-0 whitespace-pre-wrap break-all rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
        </DetailsSection>
      </div>
    </section>
  )
}

function DetailsSection({
  children,
  icon,
  title,
}: {
  children: ReactNode
  icon?: ReactNode
  title: string
}) {
  return (
    <section>
      <div className="flex items-center gap-2 text-[13px] font-semibold text-muted-foreground [&_svg]:size-3.5">
        {icon}
        <h2>{title}</h2>
      </div>
      <div className="mt-3 grid min-w-0 gap-2.5">{children}</div>
    </section>
  )
}

function DetailsValue({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[13px]">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all text-right font-mono">{value}</span>
    </div>
  )
}
