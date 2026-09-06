import assert from 'node:assert/strict'
import test from 'node:test'
import { historySelectedFilePath } from './history-selection'

const files = [{ path: '/first.ts' }, { path: '/second.ts' }]

test('history selects the first available file only when the URL has no path', () => {
  assert.equal(historySelectedFilePath(undefined, undefined, false), null)
  assert.equal(historySelectedFilePath(undefined, [], false), null)
  assert.equal(historySelectedFilePath(undefined, files, false), '/first.ts')
  assert.equal(historySelectedFilePath('/second.ts', files, false), '/second.ts')
  assert.equal(historySelectedFilePath('/missing.ts', files, false), '/missing.ts')
})

test('closing a diff dismisses only the current location and explicit selection can reopen it', () => {
  assert.equal(historySelectedFilePath('/second.ts', files, true), null)
  assert.equal(historySelectedFilePath(undefined, files, true), null)
  assert.equal(historySelectedFilePath('/second.ts', files, false), '/second.ts')
})
