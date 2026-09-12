import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  elapsedDuration,
  formatDuration,
  runDisplayState,
} from './run-formatting'

describe('run formatting', () => {
  it('labels an acknowledged cancellation consistently', () => {
    assert.equal(
      runDisplayState({ cancellation_requested: true, state: 'running' }),
      'canceling',
    )
    assert.equal(
      runDisplayState({ cancellation_requested: false, state: 'running' }),
      'running',
    )
    assert.equal(
      runDisplayState({ cancellation_requested: true, state: 'canceled' }),
      'canceled',
    )
  })

  it('counts a running span up from the shared clock', () => {
    assert.equal(elapsedDuration(100, null, 145), '45s')
    assert.equal(elapsedDuration(100, 130, 999), '30s')
    assert.equal(elapsedDuration(null, null, 145), null)
  })

  it('formats durations for scanning', () => {
    assert.equal(formatDuration(44), '44s')
    assert.equal(formatDuration(184), '3m 04s')
    assert.equal(formatDuration(120), '2m')
    assert.equal(formatDuration(4_320), '1h 12m')
  })
})
