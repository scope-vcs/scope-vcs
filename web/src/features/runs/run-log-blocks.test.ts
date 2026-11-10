import assert from 'node:assert/strict'
import test from 'node:test'
import { logBlocks } from './run-log-blocks'

test('a log splits into blocks of 200 whole lines whose text joins back into the log, blank boundary lines included', () => {
  const lines = Array.from({ length: 401 }, (_, index) => index === 199 || index === 200 ? '' : `line ${index}`)
  const blocks = logBlocks(`${lines.join('\n')}\n`)
  assert.deepEqual(blocks.map((block) => block.lineCount), [200, 200, 1])
  assert.equal(blocks.map((block) => block.text).join(''), lines.join('\n'))
  assert.equal(blocks[0]!.text.endsWith('line 198\n\n'), true)
  assert.deepEqual(logBlocks('no newline'), [{ text: 'no newline', lineCount: 1 }])
  assert.deepEqual(logBlocks(''), [])
})
