import assert from 'node:assert/strict'
import test from 'node:test'

// Formatters are built when the module loads, so the viewer's zone is chosen
// before date-format is imported.
process.env.TZ = 'America/New_York'

test('hydrated dates render in the viewer zone the UTC render fell back from', async () => {
  const {
    formatUnixDayLabel,
    formatUnixMonthDay,
    formatUnixSnoozeUntil,
    unixCalendarDay,
  } = await import('./date-format')

  assert.equal(formatUnixMonthDay(0, true), 'Dec 31')
  assert.equal(formatUnixMonthDay(0, false), 'Jan 1')
  assert.equal(formatUnixDayLabel(0, true), 'December 31, 1969')
  assert.equal(formatUnixDayLabel(0, false), 'January 1, 1970')
  assert.equal(formatUnixSnoozeUntil(0, true), 'Dec 31, 7:00 PM')
  assert.equal(formatUnixSnoozeUntil(0, false), 'Jan 1, 12:00 AM')
  assert.equal(unixCalendarDay(0, true), '1969-11-31')
  assert.equal(unixCalendarDay(0, false), '1970-0-1')
})
