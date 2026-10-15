import { strict as assert } from 'node:assert'
import test from 'node:test'
import { parseCliCallbackHandoffUrl } from './cli-callback-handoff'

test('parseCliCallbackHandoffUrl only accepts explicit loopback callbacks', () => {
  for (const host of ['127.0.0.1', 'localhost', '[::1]']) {
    const callback = `http://${host}:61353/scope-cli-callback?request_id=cli_browser_123`
    assert.equal(parseCliCallbackHandoffUrl(callback), callback)
  }
  for (const callback of [
    'https://scopevcs.com/scope-cli-callback?request_id=cli_browser_123',
    'http://example.com:61353/scope-cli-callback?request_id=cli_browser_123',
    'http://127.0.0.1/scope-cli-callback?request_id=cli_browser_123',
    'http://127.0.0.1:61353/other?request_id=cli_browser_123',
  ]) assert.throws(() => parseCliCallbackHandoffUrl(callback))
})
