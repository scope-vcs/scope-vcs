import assert from 'node:assert/strict'
import test from 'node:test'
import { readAnsi } from './run-log-ansi'

const ESC = '\u001b'

function styled(raw: string) {
  const { text, spans } = readAnsi(raw)
  return { text, spans: spans?.map(({ text: part, style }) => [part, style.tone, style.bright, style.bold, style.dim]) ?? null }
}

test('SGR colors become styled spans and reset returns to plain text', () => {
  assert.deepEqual(styled(`${ESC}[32m✓${ESC}[0m ok ${ESC}[1;91mfail${ESC}[22m!${ESC}[39m ${ESC}[2mdim${ESC}[m`), {
    text: '✓ ok fail! dim',
    spans: [
      ['✓', 'green', false, false, false],
      [' ok ', null, false, false, false],
      ['fail', 'red', true, true, false],
      ['!', 'red', true, false, false],
      [' ', null, false, false, false],
      ['dim', null, false, false, true],
    ],
  })
})

test('256-color and truecolor foregrounds map to the nearest of the 16 colors and backgrounds are dropped', () => {
  assert.deepEqual(styled(`${ESC}[38;5;196ma${ESC}[38;5;34mb${ESC}[38;2;10;10;230mc${ESC}[48;5;196;38;5;250md${ESC}[48;2;1;2;3me`).spans, [
    ['a', 'red', true, false, false],
    ['b', 'green', false, false, false],
    ['c', 'blue', false, false, false],
    ['de', 'white', false, false, false],
  ])
})

test('escapes other than SGR are removed without styling and plain lines skip parsing', () => {
  assert.deepEqual(
    styled(`${ESC}[2K${ESC}[1Gprogress ${ESC}]8;;https://example.com\u0007link${ESC}]8;;${ESC}\\ ${ESC}(Bdone${ESC}=`),
    { text: 'progress link done', spans: null },
  )
  assert.deepEqual(styled('plain text'), { text: 'plain text', spans: null })
})
