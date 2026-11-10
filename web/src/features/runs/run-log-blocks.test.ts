import assert from 'node:assert/strict'
import test from 'node:test'
import { logBlocks } from './run-log-blocks'

test('a log splits into blocks of 200 whole lines without losing or adding one', () => {
  const lines = Array.from({ length: 401 }, (_, index) => `line ${index}`)
  const blocks = logBlocks(`${lines.join('\n')}\n`)
  assert.deepEqual(blocks.map((block) => block.length), [200, 200, 1])
  assert.deepEqual(blocks.flat(), lines)
  assert.deepEqual(logBlocks('no newline'), [['no newline']])
  assert.deepEqual(logBlocks(''), [])
})
