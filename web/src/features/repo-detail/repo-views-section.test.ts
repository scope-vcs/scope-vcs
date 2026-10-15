import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { repoViews } from '../../api/repo-views'
import { RepositoryViewsSection } from './repo-views-section'

test('view descriptions distinguish public readers from assigned members', () => {
  for (const [readers, expected, unexpected] of [
    ['anyone', 'Readable by anyone', 'Readable by assigned members'],
    ['assigned', 'Readable by assigned members', 'Readable by anyone'],
  ] as const) {
    const html = renderToStaticMarkup(createElement(RepositoryViewsSection, {
      views: repoViews([{ id: 'review', name: 'Review', includes: [], readers }]),
    }))
    assert.ok(html.includes(expected), readers)
    assert.ok(!html.includes(unexpected), readers)
  }
})
