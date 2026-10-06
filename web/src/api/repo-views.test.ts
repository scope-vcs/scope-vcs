import assert from 'node:assert/strict'
import test from 'node:test'
import { parseViewId, repoViews } from './repo-views'
import type { ViewDefinition } from './types.generated'

const definitions: ViewDefinition[] = [
  { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
  { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
  { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' },
  { id: 'review', name: 'Review', includes: ['agent'], readers: 'assigned' },
  { id: 'design', name: 'Design', includes: [], readers: 'assigned' },
]
const views = repoViews(definitions)

test('readers see their view and everything it includes, transitively, in definition order', () => {
  assert.deepEqual(views.readableBy('public').map((view) => view.id), ['public'])
  assert.deepEqual(views.readableBy('review').map((view) => view.id), ['public', 'agent', 'review'])
  assert.deepEqual(views.readableBy('design').map((view) => view.id), ['design'])
  assert.deepEqual(
    views.readableBy('private').map((view) => view.id),
    ['public', 'private', 'agent', 'review', 'design'],
  )
})

test('may-read refuses views outside the reader and views the repository does not define', () => {
  assert.equal(views.mayRead('review', 'public'), true)
  assert.equal(views.mayRead('agent', 'review'), false)
  assert.equal(views.mayRead('agent', 'design'), false)
  assert.equal(views.mayRead('private', 'missing'), false)
  assert.equal(views.mayRead('missing', 'public'), false)
})

test('names come from the definitions and fall back to the id for unknown views', () => {
  assert.equal(views.name('agent'), 'Agent')
  assert.equal(views.name('removed'), 'removed')
  assert.equal(views.full, 'private')
  assert.equal(views.anyone, 'public')
  assert.deepEqual(views.includedNames('review'), ['Agent'])
  assert.deepEqual(views.includedNames('public'), [])
  assert.equal(views.includedNames('private'), 'all')
})

test('a repository without an anyone view has no anonymous view', () => {
  const closed = repoViews(definitions.filter((view) => view.readers !== 'anyone'))
  assert.equal(closed.anyone, null)
  assert.equal(closed.mayRead('agent', 'public'), false)
})

test('view ids follow the domain grammar', () => {
  assert.equal(parseViewId('agent_2'), 'agent_2')
  assert.throws(() => parseViewId('Agent'))
  assert.throws(() => parseViewId('2agent'))
  assert.throws(() => parseViewId(''))
  assert.throws(() => parseViewId(undefined))
})
