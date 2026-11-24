import type { RepoParams } from '@/api/types'
import type { LinkedRequestResponse } from '@/api/types.generated'
import { useId } from 'react'
import { LinkedRequestLink } from './request-fact-link'
import { REQUEST_LINK_WORDS, requestStatusMark, requestStatusPhrase } from './request-labels'
import { RequestStateMark } from './request-state-mark'

export function RequestResolvedBy({
  params,
  requests,
}: {
  params: RepoParams
  requests: readonly LinkedRequestResponse[]
}) {
  const headingId = useId()
  if (requests.length === 0) return null
  return (
    <section
      aria-labelledby={headingId}
      className="mt-2 -mb-px border-y border-border px-(--app-inset) py-2.5 text-xs leading-5"
    >
      <h2 className="text-muted-foreground" id={headingId}>{REQUEST_LINK_WORDS.resolvedBy}</h2>
      <ul className="mt-1">
        {requests.map((request) => (
          <li className="flex min-w-0 items-start gap-2.5 py-[3px]" key={request.id}>
            <RequestStateMark className="mt-[3px] size-3.5" mark={requestStatusMark(request.status)} />
            <span className="flex min-w-0 flex-wrap gap-x-2.5 [overflow-wrap:anywhere]">
              <LinkedRequestLink className="text-foreground" params={params} request={request} />
              <span className="text-muted-foreground">
                {request.author ? `${request.author.handle}, ` : null}
                {requestStatusPhrase(request.status)}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}
