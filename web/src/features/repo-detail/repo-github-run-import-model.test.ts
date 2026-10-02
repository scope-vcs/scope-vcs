import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubRunImportResponse } from '../../api/types.generated'
import { githubRunImportView, parseRunImportCount } from './repo-github-run-import-model'

function runImport(overrides: Partial<GitHubRunImportResponse> = {}): GitHubRunImportResponse {
  return {
    state: 'queued',
    run_count: 50,
    imported_count: 0,
    error: null,
    queued_at_unix: 10,
    finished_at_unix: null,
    ...overrides,
  }
}

test('only whole counts from 0 to 1000 can be imported', () => {
  assert.equal(parseRunImportCount('50'), 50)
  assert.equal(parseRunImportCount(' 0 '), 0)
  assert.equal(parseRunImportCount('1000'), 1000)
  for (const draft of ['', '1001', '-1', '2.5', '1e2', 'fifty']) {
    assert.equal(parseRunImportCount(draft), null, draft)
  }
})

test('an import says how far it came and what GitHub answered', () => {
  assert.equal(githubRunImportView(null), null)
  assert.deepEqual(githubRunImportView(runImport()), {
    inProgress: true,
    retrying: false,
    failed: false,
    status: 'Importing up to 50 runs from GitHub.',
  })
  assert.equal(
    githubRunImportView(runImport({ state: 'running', run_count: 1 }))?.status,
    'Importing up to 1 run from GitHub.',
  )
  assert.deepEqual(
    githubRunImportView(runImport({ error: 'GitHub answered 502 Bad Gateway: Server Error.' })),
    {
      inProgress: true,
      retrying: true,
      failed: true,
      status: 'Import failed: GitHub answered 502 Bad Gateway: Server Error. Retrying.',
    },
  )
  assert.equal(
    githubRunImportView(runImport({ state: 'succeeded', imported_count: 50, finished_at_unix: 20 }))?.status,
    'Imported 50 runs.',
  )
  assert.equal(
    githubRunImportView(runImport({ state: 'succeeded', imported_count: 0, finished_at_unix: 20 }))?.status,
    'GitHub had no runs to import.',
  )
  const failed = githubRunImportView(
    runImport({ state: 'failed', error: 'GitHub answered 403 Forbidden', finished_at_unix: 20 }),
  )
  assert.deepEqual(failed, {
    inProgress: false,
    retrying: false,
    failed: true,
    status: 'Import failed: GitHub answered 403 Forbidden.',
  })
})
