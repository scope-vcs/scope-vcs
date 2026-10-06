import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { repoViews } from '../../api/repo-views'
import { permissionSummaryText, permissionsWithView } from './repo-member-permission-model'
import { MemberViewField } from './repo-member-view-field'

const views = repoViews([
  { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
  { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
  { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' },
])

function render(view: string) {
  return renderToStaticMarkup(createElement(MemberViewField, {
    onChange: () => undefined,
    permissions: { can_change_file_visibility: false, can_push: false, view },
    views,
  }))
}

test('the member view picker offers every repository view by name', () => {
  const html = render('agent')
  assert.deepEqual([...html.matchAll(/<option[^>]*value="([^"]+)"[^>]*>([^<]+)</g)].map((match) => [match[1], match[2]]), [
    ['public', 'Public'],
    ['private', 'Private'],
    ['agent', 'Agent'],
  ])
  assert.match(html, /<option[^>]*value="agent"[^>]*selected/)
  assert.match(html, /<label[^>]*for="([^"]+)"[^>]*>View<\/label><select[^>]*id="\1"/)
})

test('a narrower view explains that pushing needs the full view', () => {
  assert.match(render('agent'), /Pushing and changing file visibility need the Private view/)
  assert.doesNotMatch(render('private'), /need the/)
})

test('moving a member to a narrower view drops the actions that need the full view', () => {
  const full = { can_change_file_visibility: true, can_push: true, view: 'private' }
  assert.deepEqual(permissionsWithView(full, 'agent', views), { can_change_file_visibility: false, can_push: false, view: 'agent' })
  assert.deepEqual(permissionsWithView({ ...full, view: 'agent', can_push: false, can_change_file_visibility: false }, 'private', views), {
    can_change_file_visibility: false,
    can_push: false,
    view: 'private',
  })
  assert.equal(permissionSummaryText(full, views), 'Private view · Also allowed: change file visibility, push changes')
  assert.equal(permissionSummaryText({ ...full, view: 'agent', can_push: false, can_change_file_visibility: false }, views), 'Agent view · No extra actions')
})
