import assert from 'node:assert/strict'
import test from 'node:test'
import type { CommitFileResponse, FileLineChangesResponse } from '@/api/types.generated'
import { abbreviatedCount, diffStatLabel, diffStatOfFiles, exactCount, fileLinesLabel } from './diff-stat'

test('counts abbreviate at a thousand without rounding up past the exact value', () => {
  const cases: [count: number, expected: string][] = [
    [0, '0'],
    [999, '999'],
    [1_000, '1k'],
    [1_050, '1k'],
    [1_999, '1.9k'],
    [2_400, '2.4k'],
    [99_999, '99.9k'],
    [100_000, '100k'],
    [999_999, '999k'],
    [1_200_000, '1.2m'],
    [250_000_000, '250m'],
  ]
  for (const [count, expected] of cases) assert.equal(abbreviatedCount(count), expected, String(count))
})

test('exact counts group thousands', () => {
  assert.equal(exactCount(1_280), '1,280')
  assert.equal(exactCount(41), '41')
})

test('labels give exact numbers and name binary-only changes', () => {
  assert.equal(
    diffStatLabel({ additions: 1_280, deletions: 41, changed_files: 6 }),
    '1,280 additions, 41 deletions across 6 files',
  )
  assert.equal(
    diffStatLabel({ additions: 1, deletions: 0, changed_files: 1 }),
    '1 addition, 0 deletions across 1 file',
  )
  assert.equal(
    diffStatLabel({ additions: 0, deletions: 0, changed_files: 3 }),
    '3 files changed, no line changes',
  )
  assert.equal(fileLinesLabel({ kind: 'binary' }), 'Binary file')
  assert.equal(fileLinesLabel({ kind: 'text', additions: 22, deletions: 1 }), '22 additions, 1 deletion')
})

test('file totals sum text lines and count binary files without lines', () => {
  const file = (path: string, lines: FileLineChangesResponse): CommitFileResponse => ({
    path,
    kind: 'Modified',
    old_mode: null,
    new_mode: null,
    old_oid: null,
    new_oid: null,
    label: 'public',
    lines,
  })
  assert.deepEqual(
    diffStatOfFiles([
      file('a.ts', { kind: 'text', additions: 22, deletions: 4 }),
      file('logo.png', { kind: 'binary' }),
      file('b.ts', { kind: 'text', additions: 3, deletions: 10 }),
    ]),
    { additions: 25, deletions: 14, changed_files: 3 },
  )
})
