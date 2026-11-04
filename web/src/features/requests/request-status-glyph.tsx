import type { RequestQueueNextStep } from '@/api/types.generated'
import { AlarmClock, CircleCheck, CircleDashed, CirclePlay, CircleX, Eye, MessageSquare, RefreshCw, Send, type LucideIcon } from 'lucide-react'
import './request-status-glyph.css'

const NEXT_STEP_ICONS: Record<RequestQueueNextStep, { icon: LucideIcon; round: boolean }> = {
  review: { icon: Eye, round: false },
  reply: { icon: MessageSquare, round: false },
  revisit: { icon: AlarmClock, round: false },
  merge: { icon: CircleCheck, round: true },
  fix_checks: { icon: CircleX, round: true },
  approve_checks: { icon: CirclePlay, round: true },
  update: { icon: RefreshCw, round: false },
  submit: { icon: Send, round: false },
  wait: { icon: CircleDashed, round: true },
  done: { icon: CircleCheck, round: true },
}

export function RequestStatusGlyph({ className, nextStep }: { className: string; nextStep: RequestQueueNextStep }) {
  const { icon: StepIcon, round } = NEXT_STEP_ICONS[nextStep]
  return <StepIcon aria-hidden="true" className={className} data-round={round || undefined} data-step={nextStep} />
}
