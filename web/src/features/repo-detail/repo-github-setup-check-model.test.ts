import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubSetupCheckResponse } from '../../api/types.generated'
import { githubSetupCheckView } from './repo-github-setup-check-model'

const NO_WORKFLOWS =
  'No workflows started. Check that your workflows include the scope/** push trigger.'

function check(overrides: Partial<GitHubSetupCheckResponse> = {}): GitHubSetupCheckResponse {
  return {
    branch: 'scope/setup-check',
    commit_oid: 'abcdef1234567890abcdef1234567890abcdef12',
    state: 'waiting',
    started_at_unix: 10,
    finished_at_unix: null,
    check_names: [],
    message: null,
    ...overrides,
  }
}

test('no test has run yet', () => {
  assert.equal(githubSetupCheckView(null), null)
})

test('a running test shows its branch and revision', () => {
  const view = githubSetupCheckView(check({ check_names: ['lint', 'test'] }))
  assert.equal(view?.running, true)
  assert.equal(
    view?.status,
    'Sent main (abcdef1) to scope/setup-check. Waiting for workflows to finish.',
  )
})

test('a finished test offers its checks', () => {
  const view = githubSetupCheckView(
    check({ state: 'finished', finished_at_unix: 20, check_names: ['test'] }),
  )
  assert.equal(view?.running, false)
  assert.equal(view?.problem, null)
  assert.equal(view?.status, 'Workflows ran on main (abcdef1). Select the results required before merge.')
})

test('a test without workflows says to add the trigger', () => {
  const view = githubSetupCheckView(
    check({ state: 'finished', finished_at_unix: 20, message: NO_WORKFLOWS }),
  )
  assert.equal(view?.problem, NO_WORKFLOWS)
})

test('a refused push shows what GitHub answered', () => {
  const view = githubSetupCheckView(
    check({ state: 'failed', finished_at_unix: 20, message: 'GitHub refused the push: ruleset' }),
  )
  assert.equal(view?.running, false)
  assert.equal(view?.status, 'The test could not send main (abcdef1) to GitHub.')
  assert.equal(view?.problem, 'GitHub refused the push: ruleset')
})
