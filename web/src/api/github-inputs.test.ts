import assert from 'node:assert/strict'
import test from 'node:test'
import {
  parseConnectRepoGitHubInput,
  parseRepoGitHubAuthorizeInput,
  parseRepoGitHubWorkflowRunsInput,
  parseSetRepoGitHubRunImportCountInput,
} from './github-inputs'

test('authorizing GitHub carries the page origin when there is one', () => {
  assert.deepEqual(
    parseRepoGitHubAuthorizeInput({ owner: 'owner', repo: 'repo', web_origin: 'https://dev.tail0000.ts.net:4443' }),
    { owner: 'owner', repo: 'repo', web_origin: 'https://dev.tail0000.ts.net:4443' },
  )
  for (const web_origin of [undefined, null, '', 42]) {
    assert.equal(parseRepoGitHubAuthorizeInput({ owner: 'owner', repo: 'repo', web_origin }).web_origin, null)
  }
})

test('connecting and changing the count carry a run count GitHub can be asked for', () => {
  const connect = { owner: 'owner', repo: 'repo', grant: 'grant', github_repository_id: 42 }
  assert.deepEqual(parseConnectRepoGitHubInput({ ...connect, acknowledge_public: true, run_import_count: 0 }), {
    ...connect,
    acknowledge_public: true,
    run_import_count: 0,
  })
  assert.equal(parseSetRepoGitHubRunImportCountInput({ owner: 'owner', repo: 'repo', count: 1000 }).count, 1000)
  for (const count of [undefined, -1, 1001, 2.5, '50']) {
    assert.throws(() => parseConnectRepoGitHubInput({ ...connect, run_import_count: count }), /between 0 and 1000/)
    assert.throws(() => parseSetRepoGitHubRunImportCountInput({ owner: 'owner', repo: 'repo', count }), /between 0 and 1000/)
  }
})

test('a page of GitHub runs names its workflow and cursor only when given', () => {
  assert.deepEqual(parseRepoGitHubWorkflowRunsInput({ owner: 'owner', repo: 'repo' }), {
    owner: 'owner', repo: 'repo', workflow: undefined, after: undefined,
  })
  assert.deepEqual(
    parseRepoGitHubWorkflowRunsInput({ owner: 'owner', repo: 'repo', workflow: 'ci / test', after: '10.7' }),
    { owner: 'owner', repo: 'repo', workflow: 'ci / test', after: '10.7' },
  )
  assert.throws(() => parseRepoGitHubWorkflowRunsInput({ owner: 'owner', repo: 'repo', workflow: '' }))
})
