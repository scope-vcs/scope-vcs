import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { repoViews } from '../../api/repo-views'
import {
  permissionAvailable,
  permissionDescription,
  permissionSummaryText,
  permissionsWithView,
} from './repo-member-permission-model'
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
  assert.match(html, /<label[^>]*for="([^"]+)"[^>]*>View<\/label><span[^>]*><select[^>]*id="\1"/)
})

test('a narrower view explains that only changing file visibility needs the full view', () => {
  assert.match(render('agent'), /Changing file visibility needs the Private view/)
  assert.doesNotMatch(render('agent'), /Pushing/)
  assert.doesNotMatch(render('private'), /needs the/)
})

test('a narrower member keeps push and lands main pushes as requests in its view', () => {
  const full = { can_change_file_visibility: true, can_push: true, view: 'private' }
  assert.deepEqual(permissionsWithView(full, 'agent', views), { can_change_file_visibility: false, can_push: true, view: 'agent' })
  assert.deepEqual(permissionsWithView({ ...full, view: 'agent', can_change_file_visibility: false }, 'private', views), {
    can_change_file_visibility: false,
    can_push: true,
    view: 'private',
  })
  assert.equal(permissionAvailable('can_push', 'agent', views), true)
  assert.equal(permissionAvailable('can_change_file_visibility', 'agent', views), false)
  assert.equal(permissionAvailable('can_change_file_visibility', 'private', views), true)
  assert.equal(permissionDescription('can_push', 'private', views), 'Allows Git pushes to this repository.')
  assert.equal(
    permissionDescription('can_push', 'agent', views),
    'Allows Git pushes. Pushes to main land as an auto-merged request in this view.',
  )
  assert.equal(permissionSummaryText(full, views), 'Private view · Also allowed: change file visibility, push changes')
  assert.equal(permissionSummaryText({ ...full, view: 'agent', can_change_file_visibility: false }, views), 'Agent view · Also allowed: push changes as requests')
  assert.equal(permissionSummaryText({ ...full, view: 'agent', can_push: false, can_change_file_visibility: false }, views), 'Agent view · No extra actions')
})
