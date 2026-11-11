import assert from 'node:assert/strict'
import test from 'node:test'
import { readLogLines, readStepLogLines, visibleLogLines } from './run-log-lines'
import { findLogMatches, logLinePieces } from './run-log-search'

const ESC = '\u001b'

test('workflow markers set line kinds, hide their text, and fold groups by their header line', () => {
  const lines = readLogLines([
    '﻿##[group]Run pnpm test',
    `${ESC}[36;1mpnpm test${ESC}[0m`,
    'ok 1\r',
    '##[endgroup]',
    '##[command]/usr/bin/git fetch',
    `${ESC}[31m##[error]Process completed with exit code 1.${ESC}[0m`,
    '##[warning]Node 16 is deprecated',
    '##[group]Empty',
    '##[group]Next',
    'inside',
    '',
  ].join('\n'))
  assert.deepEqual(
    lines.map(({ number, text, kind, group, groupSize }) => [number, text, kind, group, groupSize]),
    [
      [1, 'Run pnpm test', 'group', null, 2],
      [2, 'pnpm test', 'output', 1, 0],
      [3, 'ok 1', 'output', 1, 0],
      [5, '/usr/bin/git fetch', 'command', null, 0],
      [6, 'Process completed with exit code 1.', 'error', null, 0],
      [7, 'Node 16 is deprecated', 'warning', null, 0],
      [8, 'Empty', 'group', null, 0],
      [9, 'Next', 'group', null, 1],
      [10, 'inside', 'output', 9, 0],
    ],
  )
  assert.equal(lines[4]!.spans?.[0]?.text, 'Process completed with exit code 1.')
  assert.deepEqual(visibleLogLines(lines, new Set([9])).map((line) => line.number), [1, 5, 6, 7, 8, 9, 10])
})

test('a color set on one line stays on later lines and native chunks until reset', () => {
  const lines = readStepLogLines([
    { position: 1, text: `${ESC}[31mfirst\nsec` },
    { position: 2, text: `ond${ESC}[0m tail\nplain\n` },
  ])
  assert.deepEqual(
    lines.map((line) => line.spans?.map(({ text, style }) => [text, style.tone]) ?? null),
    [[['first', 'red']], [['second', 'red'], [' tail', null]], null],
  )
})

test('log search matches case-insensitively and highlights across color spans', () => {
  const lines = readLogLines(`${ESC}[31mAssert${ESC}[0mionError: boom\nassertionerror again\n`)
  const matches = findLogMatches(lines, 'ASSERTIONERROR')
  assert.deepEqual(matches, [{ line: 1, start: 0, end: 14 }, { line: 2, start: 0, end: 14 }])
  assert.deepEqual(
    logLinePieces(lines[0]!, [matches[0]!], matches[0]!).map(({ text, style, match }) => [text, style?.tone ?? null, match]),
    [['Assert', 'red', 'current'], ['ionError', null, 'current'], [': boom', null, null]],
  )
  assert.deepEqual(
    logLinePieces(lines[1]!, [matches[1]!], matches[0]!).map(({ text, match }) => [text, match]),
    [['assertionerror', 'other'], [' again', null]],
  )
  assert.deepEqual(findLogMatches(lines, ''), [])
  assert.deepEqual(findLogMatches(readLogLines('\u0130ERROR then error\n'), 'error'), [{ line: 1, start: 1, end: 6 }, { line: 1, start: 12, end: 17 }])
  assert.deepEqual(findLogMatches(readLogLines('axb) (a.b)\n'), 'a.b)'), [{ line: 1, start: 6, end: 10 }])
})
