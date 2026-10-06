import { Badge, type BadgeVariant } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { VisibilityState } from '@/api/types'
import { builtinViews, viewName } from '@/api/repo-views'
import { Blend, Globe2, Lock, type LucideIcon } from 'lucide-react'

function visibilityPresentation(visibility: VisibilityState): { icon: LucideIcon; variant: BadgeVariant } {
  if (visibility === 'Mixed') return { icon: Blend, variant: 'neutral' }
  if (visibility === 'public') return { icon: Globe2, variant: 'success' }
  return { icon: Lock, variant: 'neutral' }
}

export function VisibilityBadge({
  compact = false,
  visibility,
}: {
  compact?: boolean
  visibility: VisibilityState
}) {
  const { icon: Icon, variant } = visibilityPresentation(visibility)
  const label = visibility === 'Mixed' ? 'mixed' : viewName(visibility)
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
      {[...builtinViews.map((view) => view.id), 'Mixed'].map((visibility) => {
        const { icon: Icon } = visibilityPresentation(visibility)
        return (
          <span className="inline-flex items-center gap-1" key={visibility}>
            <Icon aria-hidden className="size-3" />
            {visibility === 'Mixed' ? 'mixed' : viewName(visibility)}
          </span>
        )
      })}
    </div>
  )
}
