import type { ViewDefinition, ViewId } from '@/api/types.generated'
import { cn } from '../lib/utils'

const VIEW_SELECT_CLASS = 'h-8 min-w-0 max-w-full truncate rounded-md border border-input bg-secondary px-2 text-sm text-foreground shadow-[var(--shadow-card)] outline-none transition-colors focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-60'

export function ViewSelect({
  className,
  disabled,
  id,
  label,
  onChange,
  value,
  views,
}: {
  className?: string
  disabled?: boolean
  id?: string
  label?: string
  onChange: (view: ViewId) => void
  value: ViewId
  views: readonly ViewDefinition[]
}) {
  return (
    <select
      aria-label={label}
      className={cn(VIEW_SELECT_CLASS, className)}
      disabled={disabled}
      id={id}
      onChange={(event) => onChange(event.target.value)}
      value={value}
    >
      {views.map((view) => <option key={view.id} value={view.id}>{view.name}</option>)}
    </select>
  )
}
