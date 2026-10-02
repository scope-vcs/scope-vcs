import assert from 'node:assert/strict'
import test from 'node:test'
import { parseRepoGitHubAuthorizeInput } from './github-inputs'

test('authorizing GitHub carries the page origin when there is one', () => {
  assert.deepEqual(
    parseRepoGitHubAuthorizeInput({ owner: 'owner', repo: 'repo', web_origin: 'https://dev.tail0000.ts.net:4443' }),
    { owner: 'owner', repo: 'repo', web_origin: 'https://dev.tail0000.ts.net:4443' },
  )
  for (const web_origin of [undefined, null, '', 42]) {
    assert.equal(parseRepoGitHubAuthorizeInput({ owner: 'owner', repo: 'repo', web_origin }).web_origin, null)
  }
})
