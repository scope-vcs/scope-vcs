import type { RequestParams, RequestRating, RequestRatings, RequestSummary } from '@/api/types'
import { GitCommitHorizontal } from 'lucide-react'
import { type ReactNode, useState, useSyncExternalStore } from 'react'
import { RequestInvitees } from './request-invitees'
import { RequestRatingsSection } from './request-ratings-section'
import type { RateRequestInput } from '@/api/requests'
import {
  requestAudienceLabel,
  requestAuthorRoleLabel,
  shortOid,
} from './request-labels'
import { RequestAbsoluteTimestamp } from './request-timestamp'
import type { RequestActionController } from './use-request-actions'

export function RequestContextRail({
  actions,
  onRate,
  params,
  ratings,
  request,
}: {
  actions: RequestActionController
  onRate: (input: RateRequestInput) => Promise<RequestRating>
  params: RequestParams
  ratings: RequestRatings
  request: RequestSummary
}) {
  const desktop = useSyncExternalStore(subscribeDesktop, isDesktop, () => false)
  const [mobileOpen, setMobileOpen] = useState(false)
  return (
    <aside className="request-context-rail min-w-0 xl:col-start-2 xl:row-start-1 xl:row-span-3 border-y border-border px-5 py-4 xl:border-y-0 xl:border-l xl:py-6">
      <details
        open={desktop || mobileOpen}
        onToggle={(event) => {
          if (!desktop && event.target === event.currentTarget) {
            setMobileOpen(event.currentTarget.open)
          }
        }}
      >
        <summary className="cursor-pointer text-sm font-medium xl:hidden">details</summary>
        <div className="grid min-w-0 gap-6 pt-5 xl:pt-0">
          <RailSection title="lifecycle">
            <RailValue label="Author" value={requestAuthorRoleLabel(request)} />
            <RailValue label="Audience" value={requestAudienceLabel(request)} />
            <RailValue
              label="Submitted"
              value={<RequestAbsoluteTimestamp value={request.submitted_at_unix} />}
            />
            {request.closed_at_unix !== null && (
              <RailValue
                label="Closed"
                value={<RequestAbsoluteTimestamp value={request.closed_at_unix} />}
              />
            )}
            {request.merged_at_unix !== null && (
              <RailValue
                label="Merged"
                value={<RequestAbsoluteTimestamp value={request.merged_at_unix} />}
              />
            )}
          </RailSection>

          <RequestInvitees actions={actions} request={request} />

          <RequestRatingsSection initial={ratings} onRate={onRate} params={params} />

          <RailSection icon={<GitCommitHorizontal />} title="git state">
            <RailValue label="Base" value={shortOid(request.base_main_oid)} />
            <RailValue label="Head" value={shortOid(request.head_oid)} />
            <pre className="mt-1 min-w-0 whitespace-pre-wrap break-all rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
          </RailSection>
        </div>
      </details>
    </aside>
  )
}

function RailSection({
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

function RailValue({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[13px]">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all text-right font-mono">{value}</span>
    </div>
  )
}

const DESKTOP_QUERY = '(min-width: 80rem)'

function isDesktop() {
  return window.matchMedia(DESKTOP_QUERY).matches
}

function subscribeDesktop(onChange: () => void) {
  const query = window.matchMedia(DESKTOP_QUERY)
  query.addEventListener('change', onChange)
  return () => query.removeEventListener('change', onChange)
}
