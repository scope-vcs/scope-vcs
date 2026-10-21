import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { repoViews } from '../../api/repo-views'
import type { ViewDefinition } from '../../api/types.generated'
import {
  parseViewingAsSearch,
  resolveViewingAs,
  viewHasMixedVisibility,
  viewingAsSearch,
} from './viewing-as'
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

test('visibility is mixed only when the view reads more than one label', () => {
  assert.equal(viewHasMixedVisibility(views, 'public'), false)
  assert.equal(viewHasMixedVisibility(views, 'agent'), true)
  assert.equal(viewHasMixedVisibility(views, 'private'), true)
})

test('the URL names the view only when it differs from the reader view', () => {
  assert.deepEqual(viewingAsSearch('agent', 'private'), { view: 'agent' })
  assert.deepEqual(viewingAsSearch('private', 'private'), { view: undefined })
  assert.deepEqual(parseViewingAsSearch({ view: 'agent' }), { view: 'agent' })
  assert.deepEqual(parseViewingAsSearch({ view: 'Not A View' }), {})
  assert.deepEqual(parseViewingAsSearch({}), {})
})

test('the picker stays hidden when the reader has a single view', () => {
  const html = renderToStaticMarkup(createElement(ViewingAsPicker, {
    onChange: () => undefined,
    options: views.readableBy('public'),
    value: 'public',
  }))
  assert.equal(html, '')
})

test('the picker trigger names the selected view it is viewing as', () => {
  const html = renderToStaticMarkup(createElement(ViewingAsPicker, {
    onChange: () => undefined,
    options: views.readableBy('private'),
    value: 'agent',
  }))
  const trigger = html.match(/<button[^>]*aria-haspopup="menu"[^>]*>([\s\S]*?)<\/button>/)?.[1] ?? ''
  assert.equal(trigger.replace(/<[^>]+>/g, ''), 'Viewing as Agent')
})
