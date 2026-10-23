import type { RequestQueueNextStep } from '@/api/types.generated'
import { AlarmClock, Check, Eye, Hourglass, MessageSquare, RefreshCw, X, type LucideIcon } from 'lucide-react'
import type { CSSProperties } from 'react'
import { actorHues } from './request-discussion-avatar'

const NEXT_STEP_ICONS: Record<Exclude<RequestQueueNextStep, 'done'>, LucideIcon> = {
  review: Eye,
  reply: MessageSquare,
  revisit: AlarmClock,
  merge: Check,
  fix_checks: X,
  update: RefreshCw,
  wait: Hourglass,
}

export function RequestRailMarker({
  handle,
  nextStep,
  unread,
}: {
  handle: string
  nextStep: RequestQueueNextStep
  unread: boolean
}) {
  const { hue, accent, angle } = actorHues(handle)
  return (
    <span
      aria-hidden="true"
      className="request-rail-marker"
      style={{ '--actor-hue': hue, '--actor-accent': accent, '--actor-angle': `${angle}deg` } as CSSProperties}
    >
      <span className="request-rail-marker-face">
        <span className="request-rail-marker-initial">{handle.slice(0, 1)}</span>
      </span>
      <RequestNextStepIcon className="request-rail-marker-step" nextStep={nextStep} />
      {unread && <span className="request-workspace-row-unread" />}
    </span>
  )
}

export function RequestNextStepIcon({ className, nextStep }: { className: string; nextStep: RequestQueueNextStep }) {
  if (nextStep === 'done') return null
  const StepIcon = NEXT_STEP_ICONS[nextStep]
  return (
    <span className={className} data-step={nextStep}>
      <StepIcon />
    </span>
  )
}
