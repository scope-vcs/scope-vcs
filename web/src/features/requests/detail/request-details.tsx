import type { RequestParams } from '@/api/types'
import type { RequestRatingResponse, RequestRatingsResponse, RequestSummaryResponse } from '@/api/types.generated'
import type { RateRequestInput } from '@/api/requests'
import { AbsoluteTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { shortOid } from '@/lib/short-oid'
import { useCachedResource } from '@/lib/use-cached-resource'
import { ChevronDown, Copy, SlidersHorizontal } from 'lucide-react'
import type { ReactNode } from 'react'
import { toast } from 'sonner'
import { useRepoViews } from '../../repo-detail/repo-layout-context'
import { requestAuthorRoleLabel, requestViewLabel } from './request-labels'
import { DetailsRow } from './request-details-layout'
import { requestShowsInvitees } from './request-lifecycle-model'
import { RequestInvitees } from './request-invitees'
import { type RatingDraft, RequestRatingsSection, useRatingDraft } from './request-ratings-section'
import { requestRatingsResource } from './request-ratings-resource'
import type { RequestActionController } from './use-request-actions'

type RequestDetailsProps = {
  actions: RequestActionController
  loadRatings: (signal: AbortSignal) => Promise<RequestRatingsResponse>
  onRate: (input: RateRequestInput) => Promise<RequestRatingResponse>
  params: RequestParams
  ratingsIdentity: string
  request: RequestSummaryResponse
}

export function RequestDetailsMenu(props: RequestDetailsProps) {
  const ratingDraft = useRatingDraft()
  return (
    <Popover
      align="auto"
      className="max-h-[calc(100dvh-8rem)] w-[min(22rem,calc(100vw-2rem))] overflow-y-auto p-0"
      label="Request details"
      renderPanel={() => <RequestDetails {...props} ratingDraft={ratingDraft} />}
      renderTrigger={(trigger) => (
        <Button size="sm" type="button" variant="secondary" {...trigger}>
          <SlidersHorizontal />
          Details
          <ChevronDown />
        </Button>
      )}
    />
  )
}

function RequestDetails({ actions, loadRatings, onRate, params, ratingDraft, ratingsIdentity, request }: RequestDetailsProps & { ratingDraft: RatingDraft }) {
  const views = useRepoViews()
  const ratings = useCachedResource({
    fallbackError: 'Participant ratings are unavailable.',
    identity: ratingsIdentity,
    load: loadRatings,
    resource: requestRatingsResource,
  })
  const reviewBase = request.base.kind === 'main' ? request.base.review_base_oid : request.base.head_oid
  const checkout = `git fetch origin\ngit switch --track origin/${request.name}`
  return (
    <div className="divide-y divide-border text-xs">
      <DetailsGroup>
        {!request.ask || request.author_user_id ? (
          <DetailsRow label="Author">{requestAuthorRoleLabel(request)}</DetailsRow>
        ) : null}
        <DetailsRow label="View">{requestViewLabel(request, views)}</DetailsRow>
        <DetailsRow label="Submitted"><AbsoluteTimestamp value={request.submitted_at_unix} /></DetailsRow>
        {request.closed_at_unix !== null && (
          <DetailsRow label="Closed"><AbsoluteTimestamp value={request.closed_at_unix} /></DetailsRow>
        )}
        {request.merged_at_unix !== null && (
          <DetailsRow label="Merged"><AbsoluteTimestamp value={request.merged_at_unix} /></DetailsRow>
        )}
      </DetailsGroup>

      {requestShowsInvitees(request, views) ? (
        <DetailsGroup>
          <RequestInvitees actions={actions} request={request} />
        </DetailsGroup>
      ) : null}

      {request.permissions.can_read_change ? (
        <DetailsGroup>
          <DetailsRow action={<CopyButton label="head commit" value={request.head_oid} />} label="Head">
            <code className="font-mono">{shortOid(request.head_oid)}</code>
          </DetailsRow>
          <DetailsRow action={<CopyButton label="review base commit" value={reviewBase} />} label="Review base">
            <code className="font-mono">{shortOid(reviewBase)}</code>
          </DetailsRow>
          <DetailsRow label="Main ancestor">
            <code className="font-mono">{shortOid(request.main_ancestor_oid)}</code>
            {request.main_ancestor_oid === reviewBase ? <span className="ml-1.5 text-muted-foreground">same as base</span> : null}
          </DetailsRow>
          <div className="mt-1.5 flex min-w-0 items-center gap-1 rounded-md bg-muted py-1 pr-1 pl-2">
            <code className="min-w-0 flex-1 truncate font-mono" title={checkout}>git switch --track origin/{request.name}</code>
            <CopyButton label="checkout commands" value={checkout} />
          </div>
        </DetailsGroup>
      ) : null}

      <DetailsGroup>
        {ratings.value ? (
          <RequestRatingsSection draft={ratingDraft} initial={ratings.value} onRate={onRate} params={params} />
        ) : (
          <DetailsRow
            action={ratings.error ? <Button className="h-6 px-1.5 text-xs" onClick={ratings.retry} size="sm" type="button" variant="ghost">Try again</Button> : null}
            label="Ratings"
          >
            <span className="text-muted-foreground" role={ratings.error ? 'alert' : 'status'}>
              {ratings.error ?? 'Loading…'}
            </span>
          </DetailsRow>
        )}
      </DetailsGroup>
    </div>
  )
}

function DetailsGroup({ children }: { children: ReactNode }) {
  return <section className="px-3 py-2">{children}</section>
}

function CopyButton({ label, value }: { label: string; value: string }) {
  async function copy() {
    try {
      await navigator.clipboard.writeText(value)
    } catch {
      toast.error('Copy failed')
      return
    }
    toast.success('Copied')
  }
  return (
    <Button aria-label={`Copy ${label}`} onClick={() => void copy()} size="icon-xs" title={`Copy ${label}`} type="button" variant="ghost">
      <Copy />
    </Button>
  )
}
