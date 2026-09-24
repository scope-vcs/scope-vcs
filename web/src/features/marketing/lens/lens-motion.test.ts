import assert from 'node:assert/strict'
import test from 'node:test'
import { lensCovers, lensZoom, restingPoint, ringOpacity, stepLens, tickPaths } from './lens-motion'

const start = { x: 0, y: 0, r: 0, rotation: 0 }
const viewport = { width: 1000, height: 800 }

test('eases toward the target and turns the ring with horizontal movement', () => {
  const next = stepLens(start, { x: 100, y: 50, r: 150 }, { instant: false, resting: false })
  assert.equal(next.x, 20)
  assert.equal(next.y, 10)
  assert.ok(next.r > 0 && next.r < 150)
  assert.equal(next.rotation, 7)
})

test('reduced motion and dragging land on the target without turning the ring', () => {
  const next = stepLens(start, { x: 100, y: 50, r: 150 }, { instant: true, resting: true })
  assert.deepEqual(next, { x: 100, y: 50, r: 150, rotation: 0 })
})

test('rests over the anchor while it is on screen, and floats in the viewport otherwise', () => {
  const page = { left: 0, top: -200 }
  assert.deepEqual(restingPoint({ left: 300, top: 400, right: 700, bottom: 440 }, page, viewport, 0, false), { x: 396, y: 644 })
  assert.deepEqual(restingPoint({ left: 300, top: -100, right: 700, bottom: -60 }, page, viewport, 0, false), { x: 620, y: 640 })
})

test('zoom and ring opacity follow the radius', () => {
  assert.equal(lensZoom(156, 156, false), 1.08)
  assert.equal(lensZoom(400, 156, false), 1)
  assert.equal(lensZoom(156, 156, true), 1)
  assert.equal(ringOpacity(0, 156), 0)
  assert.equal(ringOpacity(156, 156), 1)
  assert.equal(ringOpacity(2000, 156), 0)
})

test('draws 72 ticks with a major tick every 30 degrees', () => {
  const { minor, major } = tickPaths(100)
  assert.equal(major.split('M').length - 1, 12)
  assert.equal(minor.split('M').length - 1, 60)
})

test('covers a box only when it is on screen and near the center', () => {
  const box = { left: 100, top: 100, right: 300, bottom: 120 }
  assert.equal(lensCovers(box, 200, 150, 156, viewport), true)
  assert.equal(lensCovers(box, 200, 400, 156, viewport), false)
  assert.equal(lensCovers({ ...box, top: 900, bottom: 920 }, 200, 910, 156, viewport), false)
})
