import assert from 'node:assert/strict'
import test from 'node:test'
import { requestSpanRoute } from './request-span-route'

test('names request spans by route alias without path values', () => {
  assert.equal(
    requestSpanRoute('router', '/acme/private-app/requests/req_1/changes'),
    '/repository/request/changes',
  )
  assert.equal(requestSpanRoute('serverFn', '/_serverFn/abc123'), '/_serverFn')
  assert.equal(requestSpanRoute('router', '/not/a/known/route/at/all'), null)
})
