import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { runDurationLead } from './run-status'

describe('run status', () => {
  it('reads a duration the way the run went', () => {
    assert.equal(runDurationLead('running'), 'Running for')
    assert.equal(runDurationLead('queued'), 'Queued for')
    assert.equal(runDurationLead('succeeded'), 'Succeeded in')
    assert.equal(runDurationLead('failed'), 'Failed after')
    assert.equal(runDurationLead('skipped'), 'Skipped after')
    assert.equal(runDurationLead('failed', 'timed out'), 'Timed out after')
  })
})
