import assert from 'node:assert/strict'
import { test } from 'node:test'
import { REPOSITORY_HTML_CONTENT_SECURITY_POLICY } from '../components/repository-html'
import { clerkFrontendOrigin, contentSecurityPolicy, secureResponse } from './security-headers'

const clerkOrigin = 'https://scope-e2e.clerk.accounts.dev'
const security = { clerkOrigin, nonce: 'bm9uY2U=', production: true }

function directives(policy: string) {
  return new Map(policy.split('; ').map((directive) => {
    const [name, ...sources] = directive.split(' ')
    return [name, sources] as const
  }))
}

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
    const secured = secureResponse(response, security)
    assert.equal(response.bodyUsed, false)
    assert.equal(secured.body, response.body)
    assert.equal(secured.status, status)
    assert.equal(secured.headers.get('set-cookie'), 'session=value; Secure; HttpOnly')
    assert.equal(secured.headers.get('location'), '/sign-in')
    assert.equal(secured.headers.get('vary'), 'Accept-Encoding')
    assert.equal(secured.headers.get('content-security-policy'), contentSecurityPolicy(security))
    assert.equal(secured.headers.get('x-content-type-options'), 'nosniff')
    assert.equal(secured.headers.get('x-frame-options'), 'DENY')
    assert.equal(secured.headers.get('referrer-policy'), 'strict-origin-when-cross-origin')
    assert.equal(secured.headers.get('strict-transport-security'), 'max-age=31536000')
    assert.equal(await secured.text(), 'streamed content')
  }
})

test('preserves stricter policies and bodyless responses without enabling HSTS in development', () => {
  const development = { ...security, production: false }
  const secured = secureResponse(new Response(null, {
    status: 204,
    headers: { 'content-security-policy': "default-src 'none'" },
  }), development)
  assert.equal(secured.status, 204)
  assert.equal(secured.body, null)
  assert.equal(secured.headers.get('content-security-policy'), `default-src 'none', ${contentSecurityPolicy(development)}`)
  assert.equal(secured.headers.has('strict-transport-security'), false)
})

test('scripts run only from this origin, this response, and the Clerk instance in use', () => {
  const policy = directives(contentSecurityPolicy(security))
  assert.deepEqual(policy.get('default-src'), ["'self'"])
  assert.deepEqual(policy.get('script-src'), [
    "'self'", "'nonce-bm9uY2U='", "'wasm-unsafe-eval'", clerkOrigin, 'https://challenges.cloudflare.com', 'https://*.protect.clerk.com',
  ])
  assert.deepEqual(policy.get('frame-src'), ["'self'", 'https://challenges.cloudflare.com', 'https://*.protect.clerk.com'])
  assert.equal(policy.get('connect-src')?.includes('https://*.protect.clerk.com:*'), true)
  assert.deepEqual(policy.get('frame-ancestors'), ["'none'"])
  assert.deepEqual(policy.get('object-src'), ["'none'"])
  assert.deepEqual(policy.get('base-uri'), ["'self'"])
  for (const sources of policy.values()) {
    assert.equal(sources.includes("'unsafe-eval'"), false)
    assert.equal(sources.some((source) => source.startsWith('http://') || source.startsWith('ws://')), false)
  }
  assert.equal(policy.get('script-src')?.includes("'unsafe-inline'"), false)

  const signedOut = directives(contentSecurityPolicy({ ...security, clerkOrigin: null }))
  assert.deepEqual(signedOut.get('script-src'), ["'self'", "'nonce-bm9uY2U='", "'wasm-unsafe-eval'"])
  assert.deepEqual(signedOut.get('frame-src'), ["'self'"])
  assert.deepEqual(signedOut.get('connect-src'), ["'self'", 'https:'])
})

test('a sandboxed repository preview may load everything its own policy allows', () => {
  const policy = directives(contentSecurityPolicy(security))
  for (const directive of REPOSITORY_HTML_CONTENT_SECURITY_POLICY.split('; ')) {
    const [name, ...sources] = directive.split(' ')
    for (const source of sources.filter((value) => value !== "'none'")) {
      assert.equal(policy.get(name ?? '')?.includes(source), true, `${name} ${source}`)
    }
  }
})

test('local development may reach local services without loosening scripts', () => {
  const policy = directives(contentSecurityPolicy({ ...security, production: false }))
  assert.equal(policy.get('connect-src')?.includes('ws://localhost:*'), true)
  assert.equal(policy.get('img-src')?.includes('http://localhost:*'), true)
  assert.deepEqual(policy.get('script-src'), directives(contentSecurityPolicy(security)).get('script-src'))
})

test('the Clerk script origin comes from the publishable key and malformed keys add none', () => {
  assert.equal(clerkFrontendOrigin('pk_test_c2NvcGUtZTJlLmNsZXJrLmFjY291bnRzLmRldiQ'), clerkOrigin)
  assert.equal(clerkFrontendOrigin(`pk_live_${Buffer.from('clerk.scopevcs.com$').toString('base64')}`), 'https://clerk.scopevcs.com')
  for (const key of [undefined, '', 'sk_test_abc', `pk_test_${Buffer.from("evil.test 'unsafe-inline'$").toString('base64')}`]) {
    assert.equal(clerkFrontendOrigin(key), null)
  }
})
