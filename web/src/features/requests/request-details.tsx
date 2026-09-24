import type { RequestParams } from '@/api/types'
import type { RequestRatingResponse, RequestRatingsResponse, RequestSummaryResponse } from '@/api/types.generated'
import { shortOid } from '@/lib/short-oid'
import { cn } from '@/lib/utils'
import { DetailsSection, DetailsValue } from './request-details-layout'
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

export type RequestDetailsPlacement = 'rail' | 'tab'

type RequestDetailsProps = {
  actions: RequestActionController
  onRate: (input: RateRequestInput) => Promise<RequestRatingResponse>
  params: RequestParams
  /** Where the page is showing details right now. Only that placement mounts. */
  placement: RequestDetailsPlacement
  ratings: RequestRatingsResponse
  request: RequestSummaryResponse
}

const RequestDetailsContext = createContext<RequestDetailsProps | null>(null)

export function RequestDetailsProvider({ children, value }: { children: ReactNode; value: RequestDetailsProps }) {
  return <RequestDetailsContext value={value}>{children}</RequestDetailsContext>
}

/**
 * One stateful instance at a time: the rail and the Details tab both ask
 * for it, and the page decides which one is live from its own width.
 */
export function RequestDetails({ placement }: { placement: RequestDetailsPlacement }) {
  const context = use(RequestDetailsContext)
  if (!context) throw new Error('Request details context is unavailable')
  if (context.placement !== placement) return null
  const { actions, onRate, params, ratings, request } = context
  return (
    <div className={cn('@container min-w-0', placement === 'tab' && 'border-t border-border')}>
      <section aria-label="Request details" className="min-w-0 px-5 py-6 @md:px-6 @3xl:px-8">
        <div className="grid min-w-0 gap-x-12 gap-y-8 @3xl:grid-cols-2">
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

          <DetailsSection title="git state">
            <DetailsValue label="Base" value={shortOid(request.base_main_oid)} />
            <DetailsValue label="Head" value={shortOid(request.head_oid)} />
            <pre className="mt-1 min-w-0 whitespace-pre-wrap break-all rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
          </DetailsSection>
        </div>
      </section>
    </div>
  )
}


