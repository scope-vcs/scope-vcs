const REQUEST_DATE_FORMAT_OPTIONS = {
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  month: 'short',
  year: 'numeric',
} satisfies Intl.DateTimeFormatOptions

const MONTH_DAY_OPTIONS = {
  day: 'numeric',
  month: 'short',
} satisfies Intl.DateTimeFormatOptions

const DAY_LABEL_OPTIONS = {
  day: 'numeric',
  month: 'long',
  year: 'numeric',
} satisfies Intl.DateTimeFormatOptions

const SNOOZE_UNTIL_OPTIONS = {
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
  month: 'short',
} satisfies Intl.DateTimeFormatOptions

const CLOCK_TIME_OPTIONS = {
  hour: 'numeric',
  minute: '2-digit',
} satisfies Intl.DateTimeFormatOptions

const WEEKDAY_TIME_OPTIONS = {
  hour: 'numeric',
  minute: '2-digit',
  weekday: 'short',
} satisfies Intl.DateTimeFormatOptions

// Server and hydration render UTC so both sides agree; the browser switches to
// the viewer's zone once it owns the markup.
function zonedFormatters(options: Intl.DateTimeFormatOptions) {
  return {
    local: new Intl.DateTimeFormat('en-US', options),
    utc: new Intl.DateTimeFormat('en-US', { ...options, timeZone: 'UTC' }),
  }
}

type ZonedFormatters = ReturnType<typeof zonedFormatters>

const REQUEST_DATE = zonedFormatters(REQUEST_DATE_FORMAT_OPTIONS)
const MONTH_DAY = zonedFormatters(MONTH_DAY_OPTIONS)
const DAY_LABEL = zonedFormatters(DAY_LABEL_OPTIONS)
const SNOOZE_UNTIL = zonedFormatters(SNOOZE_UNTIL_OPTIONS)
const CLOCK_TIME = zonedFormatters(CLOCK_TIME_OPTIONS)
const WEEKDAY_TIME = zonedFormatters(WEEKDAY_TIME_OPTIONS)

const RELATIVE_FORMATTER = new Intl.RelativeTimeFormat('en-US', {
  numeric: 'auto',
})

const SECONDS_PER_MINUTE = 60
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR
export const RELATIVE_HORIZON_SECONDS = 30 * SECONDS_PER_DAY

export function formatUnixDate(unixSeconds: number | null) {
  return formatUnixDateWith(REQUEST_DATE.local, unixSeconds)
}

export function formatUnixDateUtc(unixSeconds: number | null) {
  return formatUnixDateWith(REQUEST_DATE.utc, unixSeconds)
}

/** Compact "Mar 4" for dense lists. */
export function formatUnixMonthDay(unixSeconds: number, hydrated: boolean) {
  return formatZoned(MONTH_DAY, unixSeconds, hydrated)
}

/** Day heading for grouped streams, as in "March 4, 2026". */
export function formatUnixDayLabel(unixSeconds: number, hydrated: boolean) {
  return formatZoned(DAY_LABEL, unixSeconds, hydrated)
}

/** Wording for a snooze that has not expired yet. */
export function formatUnixSnoozeUntil(unixSeconds: number, hydrated: boolean) {
  return formatZoned(SNOOZE_UNTIL, unixSeconds, hydrated)
}

/** Just the clock, "4:12 PM", for a moment the reader knows is today. */
export function formatUnixClockTime(unixSeconds: number, hydrated: boolean) {
  return formatZoned(CLOCK_TIME, unixSeconds, hydrated)
}

/** "Mon 9:00 AM" for a moment within the coming week. */
export function formatUnixWeekdayTime(unixSeconds: number, hydrated: boolean) {
  return formatZoned(WEEKDAY_TIME, unixSeconds, hydrated)
}

/** The calendar day a timestamp falls on, in the zone the viewer is reading. */
export function unixCalendarDay(unixSeconds: number, hydrated: boolean) {
  const date = new Date(unixSeconds * 1_000)
  return hydrated
    ? `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`
    : `${date.getUTCFullYear()}-${date.getUTCMonth()}-${date.getUTCDate()}`
}

function formatZoned(
  formatters: ZonedFormatters,
  unixSeconds: number,
  hydrated: boolean,
) {
  return (hydrated ? formatters.local : formatters.utc)
    .format(new Date(unixSeconds * 1_000))
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
