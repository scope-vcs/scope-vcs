import assert from 'node:assert/strict'
import test from 'node:test'
import { logBlocks } from './run-log-blocks'

test('log lines split into blocks of 200 without losing or adding one', () => {
  const lines = Array.from({ length: 401 }, (_, index) => index)
  assert.deepEqual(logBlocks(lines).map((block) => block.length), [200, 200, 1])
  assert.deepEqual(logBlocks(lines).flat(), lines)
  assert.deepEqual(logBlocks([]), [])
})
