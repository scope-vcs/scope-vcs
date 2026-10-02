import assert from 'node:assert/strict'
import test from 'node:test'
import {
  encodePendingGitHubTarget,
  githubSetupStep,
  parsePendingGitHubTarget,
} from './github-setup-model'

const target = { owner: 'owner', repo: 'repo' }

test('the setup page follows the OAuth callback before anything else', () => {
  assert.deepEqual(githubSetupStep({ code: 'code', state: 'state' }, target), {
    kind: 'callback', code: 'code', state: 'state',
  })
  assert.deepEqual(githubSetupStep({ error: 'access_denied', state: 'state' }, target), { kind: 'declined' })
  // Back from installing the app: restart authorization for the pending repository.
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
