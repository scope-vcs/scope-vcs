import assert from 'node:assert/strict'
import { mock, test } from 'node:test'
import { SIGN_IN_REQUIRED_HEADER } from '../api/sign-in-required'
import { SESSION_READY_BOUND_MS, withSessionRetry } from './session-retry-fetch'

const input = '/_serverFn/approve'
const init = { method: 'POST', body: '{"requestId":"req_1"}' }

const signInRequired = () => new Response('Sign in required.', { status: 401, headers: { [SIGN_IN_REQUIRED_HEADER]: '1' } })
const settle = () => new Promise<void>((resolve) => setImmediate(resolve))

function fakeSession(ready = false) {
  let wake = () => {}
  return {
    ready: () => ready,
    whenReady: mock.fn((boundMs: number) => ready ? Promise.resolve() : new Promise<void>((resolve) => {
      wake = resolve
      setTimeout(resolve, boundMs)
    })),
    becomeReady: () => {
      ready = true
      wake()
    },
  }
}

function scriptedSend(...responses: Response[]) {
  return mock.fn<typeof fetch>(async () => {
    const response = responses.shift()
    assert.ok(response, 'fetch was called more often than scripted')
    return response
  })
}

test('a request sent before the session was ready is sent once more after it becomes ready', async () => {
  const session = fakeSession()
  const approved = new Response('{}', { status: 200 })
  const send = scriptedSend(signInRequired(), approved)
  const pending = withSessionRetry(send, session)(input, init)
  await settle()
  assert.equal(send.mock.callCount(), 1)
  session.becomeReady()
  assert.equal(await pending, approved)
  assert.equal(send.mock.callCount(), 2)
  assert.deepEqual(send.mock.calls[1].arguments, [input, init])
  assert.deepEqual(session.whenReady.mock.calls.map((call) => call.arguments), [[SESSION_READY_BOUND_MS]])
})

test('the request is still sent once more when the bound elapses without a session', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const session = fakeSession()
  const stillSignedOut = signInRequired()
  const send = scriptedSend(signInRequired(), stillSignedOut)
  const pending = withSessionRetry(send, session)(input, init)
  await settle()
  t.mock.timers.tick(SESSION_READY_BOUND_MS - 1)
  await settle()
  assert.equal(send.mock.callCount(), 1)
  t.mock.timers.tick(1)
  assert.equal(await pending, stillSignedOut)
  assert.equal(send.mock.callCount(), 2)
})

test('a page whose session was ready at send time gets the sign-in response without waiting', async () => {
  const session = fakeSession(true)
  const signedOut = signInRequired()
  const send = scriptedSend(signedOut)
  assert.equal(await withSessionRetry(send, session)(input, init), signedOut)
  assert.equal(send.mock.callCount(), 1)
  assert.equal(session.whenReady.mock.callCount(), 0)
})

test('responses without the header pass through whatever their status', async () => {
  for (const response of [
    new Response('Sign in required.', { status: 401 }),
    new Response('boom', { status: 500 }),
    new Response('{}', { status: 200 }),
  ]) {
    const session = fakeSession()
    const send = scriptedSend(response)
    assert.equal(await withSessionRetry(send, session)(input, init), response)
    assert.equal(send.mock.callCount(), 1)
    assert.equal(session.whenReady.mock.callCount(), 0)
  }
})

test('readiness is judged when the request leaves, not when its response arrives', async () => {
  const session = fakeSession()
  const approved = new Response('{}', { status: 200 })
  const responses = [signInRequired(), approved]
  const send = mock.fn<typeof fetch>(async () => {
    session.becomeReady()
    return responses.shift() as Response
  })
  assert.equal(await withSessionRetry(send, session)(input, init), approved)
  assert.equal(send.mock.callCount(), 2)
})
