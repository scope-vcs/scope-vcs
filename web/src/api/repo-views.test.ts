import assert from 'node:assert/strict'
import test from 'node:test'
import { builtinViews, mayReadView, readableViews, viewName } from './repo-views'
import type { RepositoryAccessResponse, ViewDefinition } from './types.generated'

const access = (view: string) => ({ view }) as RepositoryAccessResponse

test('a reader can select only views included by their access view', () => {
  assert.deepEqual(readableViews(access('public')).map((view) => view.id), ['public'])
  assert.deepEqual(readableViews(access('private')).map((view) => view.id), ['public', 'private'])
  assert.equal(mayReadView(access('public'), 'private'), false)
  assert.equal(mayReadView(access('private'), 'missing'), false)
})

test('view names and transitive inclusion follow definitions', () => {
  const views: ViewDefinition[] = [
    ...builtinViews,
    { id: 'team', name: 'Team', includes: ['public'], readers: 'assigned' },
    { id: 'review', name: 'Review', includes: ['team'], readers: 'assigned' },
  ]
  assert.equal(viewName('team', views), 'Team')
  assert.equal(mayReadView(access('review'), 'public', views), true)
  assert.equal(mayReadView(access('team'), 'review', views), false)
})
