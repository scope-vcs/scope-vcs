import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { RequestDiscussionAuthorResponse } from '@/api/types.generated'
import { RequestDiscussionAuthorAvatar, RequestDiscussionAuthorSource } from './request-discussion-avatar'
import {
  discussionAuthorName,
  discussionAuthorUserId,
  discussionResolverName,
  isSameDiscussionAuthor,
} from './request-discussion-author'

const bot: RequestDiscussionAuthorResponse = {
  kind: 'github',
  account: {
    account_type: 'bot',
    avatar_url: 'https://avatars.githubusercontent.com/in/867647',
    github_user_id: 136622811,
    login: 'greptile-apps[bot]',
  },
  source_url: 'https://github.com/octo/repo/pull/1#discussion_r201',
}

const maya: RequestDiscussionAuthorResponse = { kind: 'user', handle: 'maya', id: 'user-maya' }

test('a GitHub author shows its avatar and a link to the original comment', () => {
  const source = renderToStaticMarkup(createElement(RequestDiscussionAuthorSource, { author: bot }))
  assert.match(
    source,
    /<a [^>]*href="https:\/\/github.com\/octo\/repo\/pull\/1#discussion_r201"[^>]*>via GitHub<\/a>/,
  )
  assert.match(source, /rel="noreferrer"/)
  assert.match(source, /aria-label="via GitHub: greptile-apps\[bot\]&#x27;s comment 201"/)
  const reviewSource = renderToStaticMarkup(createElement(RequestDiscussionAuthorSource, {
    author: { ...bot, source_url: 'https://github.com/octo/repo/pull/1#pullrequestreview-101' },
  }))
  assert.match(reviewSource, /aria-label="via GitHub: greptile-apps\[bot\]&#x27;s review 101"/)
  const avatar = renderToStaticMarkup(createElement(RequestDiscussionAuthorAvatar, { author: bot }))
  assert.match(avatar, /<img [^>]*src="https:\/\/avatars.githubusercontent.com\/in\/867647"/)
})

test('Scope authors keep initials and no GitHub link', () => {
  assert.equal(
    renderToStaticMarkup(createElement(RequestDiscussionAuthorSource, { author: maya })),
    '',
  )
  const avatar = renderToStaticMarkup(createElement(RequestDiscussionAuthorAvatar, { author: maya }))
  assert.doesNotMatch(avatar, /<img/)
  assert.match(avatar, />ma</)
})

test('GitHub authors are never mistaken for Scope users', () => {
  assert.equal(discussionAuthorName(bot), 'greptile-apps[bot]')
  assert.equal(discussionAuthorName(null), 'Deleted user')
  assert.equal(discussionAuthorUserId(bot), null)
  assert.equal(discussionAuthorUserId(maya), 'user-maya')
  assert.equal(isSameDiscussionAuthor(bot, { ...bot, source_url: 'https://github.com/x' }), true)
  assert.equal(isSameDiscussionAuthor(bot, maya), false)
  assert.equal(isSameDiscussionAuthor(null, null), false)
  assert.equal(discussionResolverName({ kind: 'github', account: bot.account }), 'greptile-apps[bot]')
})
