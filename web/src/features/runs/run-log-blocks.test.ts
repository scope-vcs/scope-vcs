import assert from 'node:assert/strict'
import test from 'node:test'
import { logBlocks } from './run-log-blocks'

test('log lines split into blocks of 200 without losing or adding one', () => {
  const lines = Array.from({ length: 401 }, (_, index) => index)
  assert.deepEqual(logBlocks(lines).map((block) => [block.start, block.lines.length]), [[0, 200], [200, 200], [400, 1]])
  assert.deepEqual(logBlocks(lines).flatMap((block) => block.lines), lines)
  assert.deepEqual(logBlocks([]), [])
})
