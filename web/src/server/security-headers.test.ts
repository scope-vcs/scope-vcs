import assert from 'node:assert/strict'
import { test } from 'node:test'
import { secureResponse, WEB_CONTENT_SECURITY_POLICY } from './security-headers'

test('protects all production responses while preserving headers, status and streamed body', async () => {
  for (const status of [200, 302, 404, 500]) {
    const response = new Response('streamed content', {
      status,
      headers: {
        'content-type': 'text/html',
        'set-cookie': 'session=value; Secure; HttpOnly',
        location: '/sign-in',
        vary: 'Accept-Encoding',
      },
    })
    const secured = secureResponse(response, true)
    assert.equal(response.bodyUsed, false)
    assert.equal(secured.body, response.body)
    assert.equal(secured.status, status)
    assert.equal(secured.headers.get('set-cookie'), 'session=value; Secure; HttpOnly')
    assert.equal(secured.headers.get('location'), '/sign-in')
    assert.equal(secured.headers.get('vary'), 'Accept-Encoding')
    assert.equal(secured.headers.get('content-security-policy'), WEB_CONTENT_SECURITY_POLICY)
    assert.equal(secured.headers.get('x-content-type-options'), 'nosniff')
    assert.equal(secured.headers.get('x-frame-options'), 'DENY')
    assert.equal(secured.headers.get('referrer-policy'), 'strict-origin-when-cross-origin')
    assert.equal(secured.headers.get('strict-transport-security'), 'max-age=31536000')
    assert.equal(await secured.text(), 'streamed content')
  }
})

test('preserves stricter policies and bodyless responses without enabling HSTS in development', () => {
  const secured = secureResponse(new Response(null, {
    status: 204,
    headers: { 'content-security-policy': "default-src 'none'" },
  }), false)
  assert.equal(secured.status, 204)
  assert.equal(secured.body, null)
  assert.equal(secured.headers.get('content-security-policy'), `default-src 'none', ${WEB_CONTENT_SECURITY_POLICY}`)
  assert.equal(secured.headers.has('strict-transport-security'), false)
})
