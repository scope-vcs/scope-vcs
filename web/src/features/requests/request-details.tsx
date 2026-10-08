import type { RequestParams } from '@/api/types'
import type { RequestRatingResponse, RequestRatingsResponse, RequestSummaryResponse } from '@/api/types.generated'
import { shortOid } from '@/lib/short-oid'
import { DetailsSection, DetailsValue } from './request-details-layout'
import { createContext, type ReactNode, use } from 'react'
import { RequestInvitees } from './request-invitees'
import { useCachedResource } from '@/lib/use-cached-resource'
import { requestRatingsResource } from './request-ratings-resource'
import { Button } from '@/components/ui/button'
import { RequestRatingsSection } from './request-ratings-section'
import type { RateRequestInput } from '@/api/requests'
import {
  requestViewLabel,
  requestAuthorRoleLabel,
} from './request-labels'
import { AbsoluteTimestamp } from '@/components/timestamp'
import type { RequestActionController } from './use-request-actions'
import { useRepoViews } from '../repo-detail/repo-layout-context'
import { requestShowsInvitees } from './request-lifecycle-model'

type RequestDetailsProps = {
  actions: RequestActionController
  onRate: (input: RateRequestInput) => Promise<RequestRatingResponse>
  params: RequestParams
  active: boolean
  loadRatings: (signal: AbortSignal) => Promise<RequestRatingsResponse>
  ratingsIdentity: string
  request: RequestSummaryResponse
}

type RequestDetailsContextValue = RequestDetailsProps & {
  ratings: RequestRatingsResponse | null
  ratingsError: string | null
  retryRatings: () => void
}

const RequestDetailsContext = createContext<RequestDetailsContextValue | null>(null)

export function RequestDetailsProvider({ children, value }: { children: ReactNode; value: RequestDetailsProps }) {
  const ratings = useCachedResource({
    enabled: value.active,
    fallbackError: 'Participant ratings are unavailable.',
    identity: value.ratingsIdentity,
    load: value.loadRatings,
    resource: requestRatingsResource,
  })
  return (
    <RequestDetailsContext value={{ ...value, ratings: ratings.value, ratingsError: ratings.error, retryRatings: ratings.retry }}>
      {children}
    </RequestDetailsContext>
  )
}

export function RequestDetails() {
  const context = use(RequestDetailsContext)
  const views = useRepoViews()
  if (!context) throw new Error('Request details context is unavailable')
  const { actions, onRate, params, ratings, ratingsError, retryRatings, request } = context
  return (
    <div className="@container min-w-0">
      <section aria-label="Request details" className="min-w-0 px-5 py-6 @md:px-6 @3xl:px-8">
        <div className="grid min-w-0 gap-x-12 gap-y-8 @3xl:grid-cols-2">
          <DetailsSection title="lifecycle">
            <DetailsValue label="Author" value={requestAuthorRoleLabel(request)} />
            <DetailsValue label="View" value={requestViewLabel(request, views)} />
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

          {requestShowsInvitees(request, views) ? (
            <RequestInvitees actions={actions} request={request} />
          ) : null}

          {ratings ? <RequestRatingsSection initial={ratings} onRate={onRate} params={params} /> : (
            <DetailsSection title="participant ratings">
              <p className="text-xs text-muted-foreground" role={ratingsError ? 'alert' : 'status'}>
                {ratingsError ?? 'Loading participant ratings…'}
              </p>
              {ratingsError ? <Button onClick={retryRatings} size="sm" variant="secondary">Try again</Button> : null}
            </DetailsSection>
          )}

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
