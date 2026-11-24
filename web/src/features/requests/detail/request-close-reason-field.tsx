import type { RequestCloseReason } from '@/api/types.generated'
import { REQUEST_CLOSE_REASONS, requestCloseReasonLabel } from './request-labels'

export function RequestCloseReasonField({
  legend,
  onChange,
  value,
}: {
  legend: string
  onChange: (reason: RequestCloseReason) => void
  value: RequestCloseReason | null
}) {
  return (
    <fieldset className="grid gap-1.5">
      <legend className="mb-1.5 text-foreground">{legend}</legend>
      {REQUEST_CLOSE_REASONS.map((option) => (
        <label className="flex items-center gap-2 text-foreground" key={option}>
          <input
            checked={value === option}
            className="accent-foreground"
            name="close-reason"
            onChange={() => onChange(option)}
            type="radio"
            value={option}
          />
          <span className="first-letter:uppercase">{requestCloseReasonLabel(option)}</span>
        </label>
      ))}
    </fieldset>
  )
}
