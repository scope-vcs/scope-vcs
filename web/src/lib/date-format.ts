const REQUEST_DATE_FORMAT_OPTIONS = {
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  month: 'short',
  year: 'numeric',
} satisfies Intl.DateTimeFormatOptions

const REQUEST_DATE_FORMATTER = new Intl.DateTimeFormat(
  'en-US',
  REQUEST_DATE_FORMAT_OPTIONS,
)

const REQUEST_UTC_DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  ...REQUEST_DATE_FORMAT_OPTIONS,
  timeZone: 'UTC',
})

const RELATIVE_FORMATTER = new Intl.RelativeTimeFormat('en-US', {
  numeric: 'auto',
})

const SECONDS_PER_MINUTE = 60
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR
export const RELATIVE_HORIZON_SECONDS = 30 * SECONDS_PER_DAY

export function formatUnixDate(unixSeconds: number | null) {
  return formatUnixDateWith(REQUEST_DATE_FORMATTER, unixSeconds)
}

export function formatUnixDateUtc(unixSeconds: number | null) {
  return formatUnixDateWith(REQUEST_UTC_DATE_FORMATTER, unixSeconds)
}

function formatUnixDateWith(
  formatter: Intl.DateTimeFormat,
  unixSeconds: number | null,
) {
  if (unixSeconds === null) {
    return 'Not set'
  }
  return formatter.format(new Date(unixSeconds * 1000))
}

/**
 * Relative wording for message streams, where every entry repeating the same
 * absolute timestamp reads as noise. Falls back to the absolute date once the
 * event is far enough away that "47 days ago" stops being useful.
 */
export function formatRelativeUnix(
  unixSeconds: number | null,
  nowUnix: number = Date.now() / 1_000,
) {
  if (unixSeconds === null) {
    return 'Not set'
  }
  const deltaSeconds = unixSeconds - nowUnix
  const distance = Math.abs(deltaSeconds)
  if (distance >= RELATIVE_HORIZON_SECONDS) {
    return formatUnixDate(unixSeconds)
  }
  if (distance < SECONDS_PER_MINUTE) {
    return 'just now'
  }
  if (distance < SECONDS_PER_HOUR) {
    return RELATIVE_FORMATTER.format(
      Math.trunc(deltaSeconds / SECONDS_PER_MINUTE),
      'minute',
    )
  }
  if (distance < SECONDS_PER_DAY) {
    return RELATIVE_FORMATTER.format(
      Math.trunc(deltaSeconds / SECONDS_PER_HOUR),
      'hour',
    )
  }
  return RELATIVE_FORMATTER.format(
    Math.trunc(deltaSeconds / SECONDS_PER_DAY),
    'day',
  )
}
