import type { ViewDefinition, ViewId } from '@/api/types.generated'
import { Select } from './ui/select'

export function ViewSelect({
  className,
  compact,
  disabled,
  id,
  label,
  onChange,
  value,
  views,
}: {
  className?: string
  compact?: boolean
  disabled?: boolean
  id?: string
  label?: string
  onChange: (view: ViewId) => void
  value: ViewId
  views: readonly ViewDefinition[]
}) {
  return (
    <Select
      aria-label={label}
      className={className}
      compact={compact}
      disabled={disabled}
      id={id}
      onChange={(event) => onChange(event.target.value)}
      value={value}
    >
      {views.map((view) => <option key={view.id} value={view.id}>{view.name}</option>)}
    </Select>
  )
}
