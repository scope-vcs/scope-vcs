import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import {
  formatRelativeUnix,
  RELATIVE_HORIZON_SECONDS,
  formatUnixDate,
  formatUnixDateUtc,
} from '@/lib/date-format'

/** Relative time with the reader's absolute local time on hover. */
export function RelativeTimestamp({
  className,
  value,
}: {
  className?: string
  value: number
}) {
  const hydrated = useHydrated()
  const nowUnix = useUnixClock()
  const date = new Date(value * 1_000)
  if (Number.isNaN(date.getTime())) return null

  return (
    <time
      className={className}
      dateTime={date.toISOString()}
      suppressHydrationWarning
      aria-label={formatExactDate(date, hydrated)}
      title={formatExactDate(date, hydrated)}
    >
      {Math.abs(value - nowUnix) >= RELATIVE_HORIZON_SECONDS
        ? formatCompactDate(date, hydrated)
        : formatRelativeUnix(value, nowUnix)}
    </time>
  )
}

/**
 * Absolute time that switches from deterministic UTC to browser local.
 */
export function AbsoluteTimestamp({
  className,
  prefix = '',
  compact = false,
  value,
}: {
  className?: string
  prefix?: string
  compact?: boolean
  value: number | null
}) {
  const hydrated = useHydrated()
  if (value === null) {
    return (
      <span className={className}>
        {prefix}
        {formatUnixDate(null)}
      </span>
    )
  }
  const date = new Date(value * 1_000)
  if (Number.isNaN(date.getTime())) return null

  return (
    <time
      className={className}
      dateTime={date.toISOString()}
      suppressHydrationWarning
      aria-label={`${prefix}${formatExactDate(date, hydrated)}`}
      title={formatExactDate(date, hydrated)}
    >
      {prefix}
      {compact
        ? formatCompactDate(date, hydrated)
        : hydrated ? formatUnixDate(value) : formatUnixDateUtc(value)}
    </time>
  )
}

const COMPACT_DATE = new Intl.DateTimeFormat('en-US', {
  month: 'short', day: 'numeric', year: 'numeric',
})
const COMPACT_DATE_UTC = new Intl.DateTimeFormat('en-US', {
  month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC',
})

function formatCompactDate(date: Date, hydrated: boolean) {
  return (hydrated ? COMPACT_DATE : COMPACT_DATE_UTC).format(date)
}

const EXACT_DATE_OPTIONS = {
  year: 'numeric', month: 'short', day: 'numeric',
  hour: '2-digit', minute: '2-digit', second: '2-digit', timeZoneName: 'short',
} satisfies Intl.DateTimeFormatOptions
const EXACT_DATE = new Intl.DateTimeFormat('en-US', EXACT_DATE_OPTIONS)
const EXACT_DATE_UTC = new Intl.DateTimeFormat('en-US', {
  ...EXACT_DATE_OPTIONS, timeZone: 'UTC',
})

function formatExactDate(date: Date, hydrated: boolean) {
  return (hydrated ? EXACT_DATE : EXACT_DATE_UTC).format(date)
}
