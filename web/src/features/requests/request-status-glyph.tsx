import type { RequestQueueNextStep } from '@/api/types.generated'
import { AlarmClock, CircleCheck, CircleDashed, CircleX, Eye, MessageSquare, RefreshCw, Send, type LucideIcon } from 'lucide-react'

const NEXT_STEP_ICONS: Record<RequestQueueNextStep, LucideIcon> = {
  review: Eye,
  reply: MessageSquare,
  revisit: AlarmClock,
  merge: CircleCheck,
  fix_checks: CircleX,
  update: RefreshCw,
  submit: Send,
  wait: CircleDashed,
  done: CircleCheck,
}

export function RequestStatusGlyph({ className, nextStep }: { className: string; nextStep: RequestQueueNextStep }) {
  const StepIcon = NEXT_STEP_ICONS[nextStep]
  return <StepIcon aria-hidden="true" className={className} data-step={nextStep} />
}
