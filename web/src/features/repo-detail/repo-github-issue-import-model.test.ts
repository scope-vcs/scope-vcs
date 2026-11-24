import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubIssueImportResponse } from '../../api/types.generated'
import { githubIssueImportView } from './repo-github-issue-import-model'

function issueImport(overrides: Partial<GitHubIssueImportResponse> = {}): GitHubIssueImportResponse {
  return {
    state: 'queued',
    view: 'private',
    imported_count: 0,
    already_imported_count: 0,
    skipped_issue_numbers: [],
    error: null,
    queued_at_unix: 10,
    finished_at_unix: null,
    ...overrides,
  }
}

const view = (overrides?: Partial<GitHubIssueImportResponse>) =>
  githubIssueImportView(issueImport(overrides), 'Private')

test('an issue import says where it is and what it has read so far', () => {
  assert.equal(githubIssueImportView(null, 'Private'), null)
  assert.deepEqual(view(), {
    working: true,
    failed: false,
    status: 'Waiting to import open issues into the Private view.',
    read: '',
  })
  assert.deepEqual(view({ state: 'running', imported_count: 12, already_imported_count: 1 }), {
    working: true,
    failed: false,
    status: 'Importing open issues into the Private view.',
    read: '12 issues imported. 1 issue already had a request.',
  })
  assert.deepEqual(
    view({ state: 'succeeded', imported_count: 1, skipped_issue_numbers: [5, 9], finished_at_unix: 20 }),
    {
      working: false,
      failed: false,
      status: 'Import into the Private view finished.',
      read: '1 issue imported. 2 issues could not be imported: #5, #9.',
    },
  )
  assert.equal(
    view({ state: 'succeeded', finished_at_unix: 20 })?.status,
    'GitHub had no open issues to import.',
  )
  assert.equal(
    view({
      state: 'succeeded',
      skipped_issue_numbers: Array.from({ length: 23 }, (_, index) => index + 1),
      finished_at_unix: 20,
    })?.read,
    `23 issues could not be imported: ${
      Array.from({ length: 20 }, (_, index) => `#${index + 1}`).join(', ')
    } and 3 more.`,
  )
})

test('a failed issue import gives the reason and can be started again unless it is still running', () => {
  assert.deepEqual(view({ error: 'GitHub answered 502 Bad Gateway.', imported_count: 3 }), {
    working: false,
    failed: true,
    status: 'Import failed: GitHub answered 502 Bad Gateway. Retrying.',
    read: '3 issues imported.',
  })
  assert.equal(view({ state: 'running', error: 'GitHub answered 502 Bad Gateway' })?.working, true)
  assert.deepEqual(
    view({ state: 'failed', error: 'The Scope GitHub App is missing issues (read).', finished_at_unix: 20 }),
    {
      working: false,
      failed: true,
      status: 'Import failed: The Scope GitHub App is missing issues (read).',
      read: '',
    },
  )
})
