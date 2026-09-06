import { Badge, type BadgeVariant } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { Visibility, VisibilityState } from '@/api/types'
import { Blend, Globe2, Lock, type LucideIcon } from 'lucide-react'

const visibilityPresentation = {
  Public: { icon: Globe2, variant: 'success' },
  Private: { icon: Lock, variant: 'neutral' },
  Mixed: { icon: Blend, variant: 'neutral' },
} as const satisfies Record<VisibilityState, { icon: LucideIcon; variant: BadgeVariant }>

export function VisibilityBadge({
  compact = false,
  visibility,
}: {
  compact?: boolean
  visibility: Visibility | VisibilityState
}) {
  const { icon: Icon, variant } = visibilityPresentation[visibility]
  const label = visibility.toLowerCase()
  return (
    <Badge
      aria-label={compact ? `${label} visibility` : undefined}
      className={cn(compact && 'w-5 gap-0 px-0')}
      role={compact ? 'img' : undefined}
      title={compact ? `${label} visibility` : undefined}
      variant={variant}
    >
      <Icon aria-hidden className="size-3" />
      {!compact && label}
    </Badge>
  )
}

export function VisibilityLegend() {
  return (
    <div aria-label="File visibility" className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
      {(Object.keys(visibilityPresentation) as VisibilityState[]).map((visibility) => {
        const { icon: Icon } = visibilityPresentation[visibility]
        return (
          <span className="inline-flex items-center gap-1" key={visibility}>
            <Icon aria-hidden className="size-3" />
            {visibility.toLowerCase()}
          </span>
        )
      })}
    </div>
  )
}
