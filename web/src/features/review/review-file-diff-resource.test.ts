import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import type { ReviewDiffRenderBudget, ReviewFileDiff } from '../../api/types'
import { createCachedResource } from '../../lib/cached-resource'
import {
  reviewFileDiffBudget,
  reviewFileDiffFullRender,
  startFullReviewFileDiff,
} from './review-file-diff-resource'

const IDENTITY = 'diff'
const FALLBACK = 'This file diff is unavailable.'

function diff(presentation: ReviewFileDiff['presentation']): ReviewFileDiff {
  return { kind: 'Modified', new_mode: '100644', old_mode: '100644', path: '/inventory.json', presentation }
}

const omitted = diff({ budget: 'standard', kind: 'omitted', reason: 'input' })
const rendered = diff({ html: '<div data-diff></div>', kind: 'html' })

test('a full render replaces the omitted diff, surfaces a failure for retry, and stays chosen on reopen', async () => {
  const resource = createCachedResource<ReviewFileDiff>({ maxEntries: 4 })
  const start = () => {}
  const fullRender = () => reviewFileDiffFullRender(resource.getSnapshot(IDENTITY), FALLBACK, start)
  await resource.ensure(IDENTITY, 'standard', async () => omitted)
  assert.deepEqual(fullRender(), { error: null, pending: false, start })

  let finishFull: (value: ReviewFileDiff) => void = () => {}
  const responses = [
    () => Promise.reject(new Error('Diff rendering took too long. Try again.')),
    () => new Promise<ReviewFileDiff>((resolve) => { finishFull = resolve }),
  ]
  const load = mock.fn((_budget: ReviewDiffRenderBudget, _signal: AbortSignal) =>
    responses.shift()?.() ?? Promise.reject(new Error('unexpected diff load')))

  await startFullReviewFileDiff(resource, IDENTITY, load)
  assert.equal(reviewFileDiffBudget(resource.getSnapshot(IDENTITY)), 'full')
  assert.equal(resource.peek(IDENTITY), omitted)
  assert.deepEqual(fullRender(), {
    error: 'Diff rendering took too long. Try again.',
    pending: false,
    start,
  })

  const retry = startFullReviewFileDiff(resource, IDENTITY, load)
  await Promise.resolve()
  assert.deepEqual(fullRender(), { error: null, pending: true, start })
  finishFull(rendered)
  await retry
  assert.equal(resource.peek(IDENTITY), rendered)
  assert.equal(fullRender(), null)

  const reopened = reviewFileDiffBudget(resource.getSnapshot(IDENTITY))
  await resource.ensure(IDENTITY, reopened, (signal) => load(reopened, signal))
  assert.equal(reopened, 'full')
  assert.equal(load.mock.callCount(), 2)
  assert.deepEqual(load.mock.calls.map((call) => call.arguments[0]), ['full', 'full'])
})

test('offers no full render once a full render is still too large', async () => {
  const resource = createCachedResource<ReviewFileDiff>({ maxEntries: 1 })
  await resource.ensure(IDENTITY, 'full', async () =>
    diff({ budget: 'full', kind: 'omitted', reason: 'lines' }))
  assert.equal(reviewFileDiffFullRender(resource.getSnapshot(IDENTITY), FALLBACK, () => {}), null)
})
