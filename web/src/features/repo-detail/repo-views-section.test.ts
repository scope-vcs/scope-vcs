import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { repoViews } from '../../api/repo-views'
import { RepositoryViewsSection } from './repo-views-section'

test('the views section names each view, its included views and its readers', () => {
  const html = renderToStaticMarkup(createElement(RepositoryViewsSection, {
    views: repoViews([
      { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
      { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
      { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' },
    ]),
  }))
  const rows = [...html.matchAll(/<li[^>]*>(.*?)<\/li>/g)].map((match) => match[1].replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim())
  assert.deepEqual(rows, [
    'Public public Its own files only · Readable by anyone',
    'Private private Includes every view · Readable by assigned members',
    'Agent agent Includes Public · Readable by assigned members',
  ])
  assert.match(html, /scope view/)
})
