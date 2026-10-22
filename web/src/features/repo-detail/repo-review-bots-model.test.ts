import assert from 'node:assert/strict'
import test from 'node:test'
import {
  reviewBotLogins,
  reviewBotsProblem,
  reviewBotsText,
  sameReviewBots,
} from './repo-review-bots-model'

test('review bot logins are separated by commas, spaces, or lines', () => {
  assert.deepEqual(
    reviewBotLogins(' greptile-apps[bot],chatgpt-codex-connector[bot]\n octo-reviewer[bot] ,, '),
    ['greptile-apps[bot]', 'chatgpt-codex-connector[bot]', 'octo-reviewer[bot]'],
  )
  assert.deepEqual(reviewBotLogins('  '), [])
})

test('the saved list shows one login per line and only real edits are unsaved', () => {
  const bots = [{ login: 'greptile-apps[bot]' }, { login: 'chatgpt-codex-connector[bot]' }]
  const text = reviewBotsText(bots)
  assert.equal(text, 'greptile-apps[bot]\nchatgpt-codex-connector[bot]')
  assert.equal(sameReviewBots(reviewBotLogins(text), bots), true)
  assert.equal(sameReviewBots(reviewBotLogins('greptile-apps[bot], chatgpt-codex-connector[bot]'), bots), true)
  assert.equal(sameReviewBots(['chatgpt-codex-connector[bot]', 'greptile-apps[bot]'], bots), false)
  assert.equal(sameReviewBots([], []), true)
})

test('the field names the entry the server would refuse', () => {
  assert.equal(reviewBotsProblem(['greptile-apps[bot]', 'chatgpt-codex-connector[bot]']), null)
  assert.equal(reviewBotsProblem([]), null)
  assert.equal(
    reviewBotsProblem(['greptile-apps[bot]', 'octocat']),
    '"octocat" is not a bot. Review bots are GitHub bot logins ending in [bot], like greptile-apps[bot]; people who can write to the repository are imported without being listed.',
  )
  for (const login of ['[bot]', '-octo[bot]', 'octo-[bot]', 'octo/cat[bot]', `${'a'.repeat(40)}[bot]`]) {
    assert.equal(
      reviewBotsProblem([login]),
      `"${login}" is not a GitHub bot login. Use a name like greptile-apps[bot].`,
      login,
    )
  }
  assert.equal(
    reviewBotsProblem(['greptile-apps[bot]', 'Greptile-Apps[bot]']),
    'Greptile-Apps[bot] is listed twice.',
  )
  assert.equal(
    reviewBotsProblem(Array.from({ length: 21 }, (_, index) => `bot-${index}[bot]`)),
    'A repository can list at most 20 review bots.',
  )
})
