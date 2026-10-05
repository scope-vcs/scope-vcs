import assert from 'node:assert/strict'
import { test } from 'node:test'
import { callbackAssertionViolations } from './callback-assertions.mjs'

const check = (source) => callbackAssertionViolations('src/lib/cached-resource.test.ts', source)

test('rejects the historical caught assertion and aliased sentinel callbacks', () => {
  for (const source of [
    `import assert from 'node:assert/strict'; import test from 'node:test'; test('retained', async () => { await store.ensure('large', '1', async () => assert.fail('active data must remain usable')) })`,
    `import {fail as unexpected} from 'node:assert'; import {test as scenario} from 'node:test'; scenario('retained', async () => { const load = () => unexpected('unexpected'); await store.ensure('large', '1', load) })`,
    `import * as check from 'node:assert/strict'; import {test} from 'node:test'; test('stream', async () => { await stream({onEvent: check['fail']}) })`,
  ]) {
    const violations = check(source)
    assert.equal(violations.length, 1)
    assert.match(violations[0], /^src\/lib\/cached-resource.test.ts:1:.*mock\.callCount/)
  }
})

test('accepts assertions owned by the test runner and explicit call counts', () => {
  assert.deepEqual(check(`
    import assert from 'node:assert/strict'
    import test, {mock} from 'node:test'
    test('retained', async () => {
      const load = mock.fn(async () => { throw new Error('unexpected read') })
      await store.ensure('large', '1', load)
      assert.equal(load.mock.callCount(), 0)
      if (broken) assert.fail('observed failure')
    })
    function timeout() { assert.fail('condition did not settle') }
  `), [])
})
