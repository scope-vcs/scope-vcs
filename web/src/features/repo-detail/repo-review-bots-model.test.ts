import assert from 'node:assert/strict'
import test from 'node:test'
import { reviewBotLogins, reviewBotsText, sameReviewBots } from './repo-review-bots-model'

test('review bot logins are separated by commas, spaces, or lines', () => {
  assert.deepEqual(
    reviewBotLogins(' greptile-apps[bot],chatgpt-codex-connector[bot]\n octo-reviewer[bot] ,, '),
    ['greptile-apps[bot]', 'chatgpt-codex-connector[bot]', 'octo-reviewer[bot]'],
  )
  assert.deepEqual(reviewBotLogins('  '), [])
})

test('the saved list round-trips through the field and only real edits are unsaved', () => {
  const bots = [{ login: 'greptile-apps[bot]' }, { login: 'chatgpt-codex-connector[bot]' }]
  const text = reviewBotsText(bots)
  assert.equal(text, 'greptile-apps[bot], chatgpt-codex-connector[bot]')
  assert.equal(sameReviewBots(text, bots), true)
  assert.equal(sameReviewBots('greptile-apps[bot]\nchatgpt-codex-connector[bot]', bots), true)
  assert.equal(sameReviewBots('chatgpt-codex-connector[bot], greptile-apps[bot]', bots), false)
  assert.equal(sameReviewBots('greptile-apps[bot]', bots), false)
  assert.equal(sameReviewBots('', []), true)
})
