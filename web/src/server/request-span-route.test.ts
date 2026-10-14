import assert from 'node:assert/strict'
import test from 'node:test'
import { requestSpanRoute } from './request-span-route'

test('names request spans by route alias without path values', () => {
  assert.equal(
    requestSpanRoute('/acme/private-app/requests/req_1/changes'),
    '/repository/request/changes',
  )
  assert.equal(requestSpanRoute('/_serverFn/abc123'), '/_serverFn')
  assert.equal(requestSpanRoute('/not/a/known/route/at/all'), null)
})
