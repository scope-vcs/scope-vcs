import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryVisibilityChangeResponse, ViewDefinition } from '../../api/types.generated'
import { transitionPathLabel, viewsTransitionChanges } from './views-transition-model'

const publicView: ViewDefinition = { id: 'public', name: 'Public', includes: [], readers: 'anyone' }
const privateView: ViewDefinition = { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' }
const agent: ViewDefinition = { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' }
const design: ViewDefinition = { id: 'design', name: 'Design', includes: [], readers: 'assigned' }

test('a transition lists added, renamed, re-included and removed views by name', () => {
  assert.deepEqual(
    viewsTransitionChanges({ before: [publicView, privateView], after: [publicView, privateView, agent] }),
    ['Added Agent, including Public'],
  )
  assert.deepEqual(
    viewsTransitionChanges({
      before: [publicView, privateView, agent, design],
      after: [publicView, privateView, { ...agent, name: 'Bots', includes: ['design'] }],
    }),
    ['Renamed Agent to Bots', 'Bots now includes Design', 'Bots no longer includes Public', 'Removed Design'],
  )
})

test('a transition reports reader changes and nothing for unchanged views', () => {
  assert.deepEqual(
    viewsTransitionChanges({
      before: [publicView, privateView],
      after: [{ ...publicView, readers: 'assigned' }, privateView],
    }),
    ['Public is now readable only by assigned members'],
  )
  assert.deepEqual(viewsTransitionChanges({ before: [publicView, privateView], after: [publicView, privateView] }), [])
})

test('paths that only crossed a view boundary say whether they entered or left', () => {
  const change = (kind: 'Added' | 'Deleted' | null, newLabel = 'public') => ({
    id: 'change',
    path: '/src/main.rs',
    old_label: 'public',
    new_label: newLabel,
    file: kind && { path: '/src/main.rs', kind, old_mode: null, new_mode: null, old_oid: null, new_oid: null, label: 'public' },
  }) satisfies HistoryVisibilityChangeResponse
  assert.equal(transitionPathLabel(change('Added'), 'Agent'), 'Entered Agent')
  assert.equal(transitionPathLabel(change('Deleted'), 'Agent'), 'Left Agent')
  assert.equal(transitionPathLabel(change('Added', 'agent'), 'Agent'), null)
})
