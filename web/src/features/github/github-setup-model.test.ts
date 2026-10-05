import assert from 'node:assert/strict'
import test from 'node:test'
import {
  encodePendingGitHubTarget,
  githubReturnPath,
  githubSetupStep,
  parsePendingGitHubTarget,
} from './github-setup-model'

const target = { owner: 'owner', repo: 'repo' }

test('the setup page follows the OAuth callback before anything else, then restarts authorization for the pending repository after the app is installed', () => {
  assert.deepEqual(githubSetupStep({ code: 'code', state: 'state' }, target), {
    kind: 'callback', code: 'code', state: 'state',
  })
  assert.deepEqual(githubSetupStep({ error: 'access_denied', state: 'state' }, target), { kind: 'declined' })
  assert.deepEqual(githubSetupStep({}, target), { kind: 'resume', target })
  assert.deepEqual(githubSetupStep({ code: 'code' }, null), { kind: 'incomplete' })
  assert.deepEqual(githubSetupStep({}, null), { kind: 'incomplete' })
})

test('a pending repository survives storage and malformed values are ignored', () => {
  assert.deepEqual(parsePendingGitHubTarget(encodePendingGitHubTarget(target)), target)
  for (const value of [null, '', 'not json', '"owner/repo"', '{"owner":"owner"}', '{"owner":"a/b","repo":"repo"}']) {
    assert.equal(parsePendingGitHubTarget(value), null, String(value))
  }
})

test('connecting returns to the page it started from within the same repository', () => {
  assert.equal(githubReturnPath('/owner/repo/runs', target), '/owner/repo/runs')
  assert.equal(githubReturnPath('/owner/repo/runs/workflows/ci', target), '/owner/repo/runs/workflows/ci')
  assert.equal(githubReturnPath('/owner/repo', target), '/owner/repo')
  for (const stored of [
    null,
    '',
    '/owner/other/runs',
    '/owner/repository/runs',
    '/owner/repo/../other/runs',
    '/owner/repo//evil.example',
    '/owner/repo/runs?next=https://evil.example',
    'https://evil.example/owner/repo/runs',
  ]) {
    assert.equal(githubReturnPath(stored, target), '/owner/repo/settings', String(stored))
  }
})
