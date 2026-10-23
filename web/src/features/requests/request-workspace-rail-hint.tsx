import type { RepoParams } from '@/api/types'
import type { RequestQueueNextStep } from '@/api/types.generated'
import { Link } from '@tanstack/react-router'
import type { CSSProperties } from 'react'
import { createPortal } from 'react-dom'
import { RequestNextStepIcon } from './request-workspace-rail-marker'
import './request-workspace-rail-hint.css'

const RAIL_HINT_VIEWPORT_MARGIN = 8

export type RailHintRequest = {
  id: string
  title: string
  nextStep: RequestQueueNextStep
  status: string
  age: string | undefined
  depth: number
  hovered: boolean
}

export type RailHint =
  | { title: string; details: string[] }
  | { requests: RailHintRequest[]; stack: boolean }

export function RequestRailHint({
  hint,
  left,
  params,
  top,
}: {
  hint: RailHint
  left: number
  params: RepoParams
  top: number
}) {
  return createPortal(
    <div
      aria-hidden="true"
      className="request-workspace-rail-hint"
      ref={alignHoveredRequest}
      style={{ left, top }}
    >
      {'requests' in hint ? (
        <ol className="request-workspace-rail-hint-requests" data-stack={hint.stack || undefined}>
          {hint.requests.map((request) => (
            <li
              data-hovered={request.hovered || undefined}
              data-nested={request.depth > 0 || undefined}
              key={request.id}
              style={{ '--stack-depth': request.depth } as CSSProperties}
            >
              {hint.stack ? (
                <Link
                  params={{ ...params, requestId: request.id }}
                  preload="intent"
                  search={(current) => ({ view: current.view })}
                  tabIndex={-1}
                  to="/$owner/$repo/requests/$requestId"
                >
                  <RailHintRequestLine request={request} />
                </Link>
              ) : (
                <span>
                  <RailHintRequestLine request={request} />
                </span>
              )}
            </li>
          ))}
        </ol>
      ) : (
        <>
          <span className="block font-medium">{hint.title}</span>
          {hint.details.map((detail) => (
            <span className="block text-muted-foreground" key={detail}>
              {detail}
            </span>
          ))}
        </>
      )}
    </div>,
    document.body,
  )
}

function RailHintRequestLine({ request }: { request: RailHintRequest }) {
  return (
    <>
      <span className="request-workspace-rail-hint-step-slot" title={request.status}>
        <RequestNextStepIcon className="request-workspace-rail-hint-step" nextStep={request.nextStep} />
      </span>
      <span className="request-workspace-rail-hint-title">{request.title}</span>
      {request.age && <span className="request-workspace-rail-hint-age">{request.age}</span>}
    </>
  )
}

function alignHoveredRequest(hint: HTMLDivElement | null) {
  const title = hint?.querySelector('[data-hovered] .request-workspace-rail-hint-title')
  if (!hint || !title) return
  const line = title.getBoundingClientRect()
  const hovered = line.top - hint.getBoundingClientRect().top + line.height / 2
  const anchor = hint.offsetTop
  const highest = RAIL_HINT_VIEWPORT_MARGIN - anchor
  const lowest = window.innerHeight - RAIL_HINT_VIEWPORT_MARGIN - hint.offsetHeight - anchor
  hint.style.setProperty('--hint-shift', `${Math.max(highest, Math.min(-hovered, lowest))}px`)
  hint.scrollTop = hovered - hint.clientHeight / 2
}
