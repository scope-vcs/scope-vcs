import assert from 'node:assert/strict'
import test from 'node:test'
import { autoPopoverOffset } from './popover-position'

test('an auto popover opens from its anchor, flips to its end, and otherwise stays on screen', () => {
  const panel = { anchorWidth: 80, panelWidth: 480, viewportWidth: 768 }
  assert.equal(autoPopoverOffset({ ...panel, anchorLeft: 100 }), 0)
  assert.equal(autoPopoverOffset({ ...panel, anchorLeft: 600 }), -400)
  const middle = autoPopoverOffset({ ...panel, anchorLeft: 330 })
  assert.equal(330 + middle, 768 - 480)
  assert.equal(autoPopoverOffset({ ...panel, anchorLeft: 20, viewportWidth: 320 }), -20)
})
