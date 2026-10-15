import assert from 'node:assert/strict'
import test from 'node:test'
import { lensCovers, restingPoint, stepLens } from './lens-motion'

const start = { x: 0, y: 0, r: 0, rotation: 0 }
const viewport = { width: 1000, height: 800 }

test('reduced motion and dragging land on the target without turning the ring, and still ease the radius', () => {
  const next = stepLens(start, { x: 100, y: 50, r: 150 }, { instant: true, resting: true })
  assert.deepEqual({ x: next.x, y: next.y, rotation: next.rotation }, { x: 100, y: 50, rotation: 0 })
  assert.ok(next.r > 0 && next.r < 150)
})

test('rests over the anchor while it is on screen, and floats in the viewport otherwise', () => {
  const page = { left: 0, top: -200 }
  assert.deepEqual(restingPoint({ left: 300, top: 400, right: 700, bottom: 440 }, page, viewport, 0, false), { x: 396, y: 644 })
  assert.deepEqual(restingPoint({ left: 300, top: -100, right: 700, bottom: -60 }, page, viewport, 0, false), { x: 620, y: 640 })
})

test('covers a box only when it is on screen and near the center', () => {
  const box = { left: 100, top: 100, right: 300, bottom: 120 }
  assert.equal(lensCovers(box, 200, 150, 156, viewport), true)
  assert.equal(lensCovers(box, 200, 400, 156, viewport), false)
  assert.equal(lensCovers({ ...box, top: 900, bottom: 920 }, 200, 910, 156, viewport), false)
})
