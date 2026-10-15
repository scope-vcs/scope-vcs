import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { runDurationLead, runStatus } from './run-status'

describe('run status', () => {
  it('prefers a terminal reason over the raw state', () => {
    assert.equal(
      runStatus('failed', { kind: 'timed-out', step_index: 2 }).label,
      'timed out',
    )
    assert.equal(
      runStatus('failed', { kind: 'step-failed', step_index: 1, exit_code: 2 })
        .label,
      'failed',
    )
  })

  it('reads a duration the way the run went', () => {
    assert.equal(runDurationLead('running'), 'Running for')
    assert.equal(runDurationLead('queued'), 'Queued for')
    assert.equal(runDurationLead('succeeded'), 'Succeeded in')
    assert.equal(runDurationLead('failed'), 'Failed after')
    assert.equal(runDurationLead('canceled'), 'Canceled after')
    assert.equal(runDurationLead('failed', 'timed out'), 'Timed out after')
  })
})
