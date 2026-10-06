import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { repoViews } from '../../api/repo-views'
import type { ViewDefinition } from '../../api/types.generated'
import { parseViewingAsSearch, resolveViewingAs, viewingAsSearch } from './viewing-as'
import { ViewingAsPicker } from './viewing-as-picker'

const definitions: ViewDefinition[] = [
  { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
  { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
  { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' },
]
const views = repoViews(definitions)

test('the requested view applies only when the reader may read it', () => {
  assert.equal(resolveViewingAs(views, 'private', 'agent'), 'agent')
  assert.equal(resolveViewingAs(views, 'agent', 'public'), 'public')
  assert.equal(resolveViewingAs(views, 'agent', 'private'), 'agent')
  assert.equal(resolveViewingAs(views, 'agent', 'missing'), 'agent')
  assert.equal(resolveViewingAs(views, 'agent', undefined), 'agent')
})

test('the URL names the view only when it differs from the reader view', () => {
  assert.deepEqual(viewingAsSearch('agent', 'private'), { view: 'agent' })
  assert.deepEqual(viewingAsSearch('private', 'private'), { view: undefined })
  assert.deepEqual(parseViewingAsSearch({ view: 'agent' }), { view: 'agent' })
  assert.deepEqual(parseViewingAsSearch({ view: 'Not A View' }), {})
  assert.deepEqual(parseViewingAsSearch({}), {})
})

test('the picker lists the readable views by name with the current view selected', () => {
  const html = renderToStaticMarkup(createElement(ViewingAsPicker, {
    onChange: () => undefined,
    options: views.readableBy('private'),
    value: 'agent',
  }))
  assert.match(html, /Viewing as/)
  assert.deepEqual([...html.matchAll(/<option[^>]*value="([^"]+)"[^>]*>([^<]+)</g)].map((match) => [match[1], match[2]]), [
    ['public', 'Public'],
    ['private', 'Private'],
    ['agent', 'Agent'],
  ])
  assert.match(html, /<option[^>]*value="agent"[^>]*selected/)
})

test('the picker stays hidden when the reader has a single view', () => {
  const html = renderToStaticMarkup(createElement(ViewingAsPicker, {
    onChange: () => undefined,
    options: views.readableBy('public'),
    value: 'public',
  }))
  assert.equal(html, '')
})
