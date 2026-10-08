import type { ViewDefinition, ViewId } from '@/api/types.generated'
import { ViewSelect } from '../../components/view-select'
import { cn } from '../../lib/utils'
import { Eye } from 'lucide-react'

export function ViewingAsPicker({
  className,
  compact = false,
  onChange,
  options,
  value,
}: {
  className?: string
  compact?: boolean
  onChange: (view: ViewId) => void
  options: readonly ViewDefinition[]
  value: ViewId
}) {
  if (options.length < 2) return null
  return (
    <label className={cn('inline-flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground', className)}>
      <Eye aria-hidden="true" className="size-3.5 shrink-0" />
      <span className="shrink-0">Viewing as</span>
      <ViewSelect className="max-w-40" compact={compact} onChange={onChange} value={value} views={options} />
    </label>
  )
}
