import { Badge, type BadgeVariant } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { VisibilityState } from '@/api/types'
import type { RepoViews } from '@/api/repo-views'
import { useRepoViews } from '@/features/repo-detail/repo-layout-context'
import { Blend, Globe2, Lock, UsersRound, type LucideIcon } from 'lucide-react'

type VisibilityKind = 'anyone' | 'full' | 'custom' | 'mixed'

const PRESENTATION: Record<VisibilityKind, { icon: LucideIcon; variant: BadgeVariant }> = {
  anyone: { icon: Globe2, variant: 'success' },
  full: { icon: Lock, variant: 'neutral' },
  custom: { icon: UsersRound, variant: 'info' },
  mixed: { icon: Blend, variant: 'neutral' },
}

function visibilityKind(visibility: VisibilityState, views: RepoViews): VisibilityKind {
  if (visibility === 'Mixed') return 'mixed'
  if (visibility === views.anyone) return 'anyone'
  if (visibility === views.full) return 'full'
  return 'custom'
}

export function VisibilityBadge({
  compact = false,
  visibility,
}: {
  compact?: boolean
  visibility: VisibilityState
}) {
  const views = useRepoViews()
  const { icon: Icon, variant } = PRESENTATION[visibilityKind(visibility, views)]
  const label = visibility === 'Mixed' ? 'mixed' : views.name(visibility)
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
  const views = useRepoViews()
  const custom = views.definitions.some((view) => view.id !== views.anyone && view.id !== views.full)
  const entries: { kind: VisibilityKind; label: string }[] = [
    ...(views.anyone ? [{ kind: 'anyone' as const, label: views.name(views.anyone) }] : []),
    ...(custom ? [{ kind: 'custom' as const, label: 'other views' }] : []),
    ...(views.full ? [{ kind: 'full' as const, label: views.name(views.full) }] : []),
    { kind: 'mixed', label: 'mixed' },
  ]
  return (
    <div aria-label="File visibility" className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
      {entries.map(({ kind, label }) => {
        const { icon: Icon } = PRESENTATION[kind]
        return (
          <span className="inline-flex items-center gap-1" key={kind}>
            <Icon aria-hidden className="size-3" />
            {label}
          </span>
        )
      })}
    </div>
  )
}
