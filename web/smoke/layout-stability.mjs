import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  owner,
  repoPath,
  requestRepoPath,
  waitForClientHydration,
  withPage,
} from './browser-smoke.mjs'

const VIEWPORTS = {
  desktop: { width: 1440, height: 900 },
  mobile: { width: 390, height: 844 },
}
const MOVE_TOLERANCE_PX = 2
const HELD_MS = 100
const QUIET_MS = 1_500
const SETTLE_TIMEOUT_MS = 30_000
const LATENCY_STEPS_MS = [150, 590, 370, 810, 260]

const storageState = process.env.SCOPE_SMOKE_STORAGE_STATE
const requestPath = `${requestRepoPath}/requests/req_demo_ready`

const SCENARIOS = [
  { name: 'profile', from: repoPath, to: `/${owner}` },
  { name: 'repository home', from: `/${owner}`, to: repoPath },
  { name: 'code', from: `/${owner}`, to: `${repoPath}/code` },
  { name: 'update', from: `/${owner}`, to: `${repoPath}/updates/dev-public-1` },
  { name: 'requests', from: `/${owner}`, to: `${requestRepoPath}/requests` },
  { name: 'request', from: `${requestRepoPath}/requests`, to: requestPath },
  { name: 'request changes', from: `${requestRepoPath}/requests`, to: `${requestPath}/changes` },
  ...storageState ? [
    { name: 'runs', from: repoPath, to: `${repoPath}/runs`, signedIn: true },
    { name: 'repository settings', from: `/${owner}`, to: `${repoPath}/settings`, signedIn: true },
    { name: 'account settings', from: `/${owner}`, to: '/settings', signedIn: true },
  ] : [],
]

export function testLayoutStability(viewportName) {
  const viewport = VIEWPORTS[viewportName]
  for (const scenario of SCENARIOS) {
    for (const arrival of ['load', 'navigation']) {
      const id = `${viewportName} ${scenario.name} by ${arrival}`
      test(`layout stability: ${id}`, async () => {
        const start = arrival === 'load' ? scenario.to : scenario.from
        await withPage(start, async (page) => {
          if (arrival === 'navigation') await navigate(page, scenario.to)
          await settle(page)
          const report = await page.evaluate(() => globalThis.__layoutStability.report())
          assert.deepEqual(problems(report), [], id)
        }, {
          prepare: (page) => instrument(page),
          settle: arrival === 'navigation',
          viewport,
          ...scenario.signedIn ? { storageState } : {},
        })
      })
    }
  }
}

const PAGE_END = '[data-slot="site-footer"]'

async function instrument(page) {
  let request = 0
  await page.route('**/_serverFn/**', async (route) => {
    const latency = LATENCY_STEPS_MS[request++ % LATENCY_STEPS_MS.length]
    await new Promise((resolve) => setTimeout(resolve, latency))
    await route.continue().catch(() => {})
  })
  await page.addInitScript(trackLayout, { heldMs: HELD_MS, pageEnd: PAGE_END, tolerance: MOVE_TOLERANCE_PX })
}

async function navigate(page, href) {
  await waitForClientHydration(page.locator('header.application-topbar a').first())
  await page.evaluate((next) => {
    globalThis.__layoutStability.reset()
    void globalThis.__TSR_ROUTER__.navigate({ href: next })
  }, href)
}

async function settle(page) {
  await page.waitForFunction((quietMs) => {
    const tracker = globalThis.__layoutStability
    return tracker
      && globalThis.__TSR_ROUTER__?.state.status === 'idle'
      && performance.now() - tracker.lastChange() > quietMs
  }, QUIET_MS, { timeout: SETTLE_TIMEOUT_MS, polling: 100 })
}

function problems({ blanks, moves }) {
  return [
    ...moves.map(({ at, dx, dy, path, text }) => `"${text}" moved ${dx}px, ${dy}px at ${at}ms (${path})`),
    ...blanks.map(({ at }) => `the page went blank at ${at}ms after showing content`),
  ]
}

function trackLayout({ heldMs, pageEnd, tolerance }) {
  const RESTYLE_MIN_TEXTS = 5
  const RESTYLE_SHARE = 0.75
  let start = performance.now()
  let lastChange = start
  let pending = false
  let blank = false
  let stale = new WeakSet()
  let settled = new Map()
  let candidates = new Map()
  let moves = []
  let blanks = []

  const elapsed = () => Math.round(performance.now() - start)
  const describe = (element) => {
    const parts = []
    for (let node = element; node && node !== document.body && parts.length < 4; node = node.parentElement) {
      const slot = node.getAttribute('data-slot')
      parts.unshift(`${node.tagName.toLowerCase()}${node.id ? `#${node.id}` : ''}${slot ? `[data-slot=${slot}]` : ''}`)
    }
    return parts.join(' > ')
  }
  const visibleText = () => {
    const found = new Map()
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = node.textContent.trim()
      const element = node.parentElement
      if (text.length < 3 || !element || element.closest(`[aria-hidden="true"], .sr-only, [data-slot="skeleton"], ${pageEnd}`)) continue
      if (!element.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue
      const range = document.createRange()
      range.selectNodeContents(node)
      const rect = range.getBoundingClientRect()
      if (!rect.width || !rect.height || rect.bottom < 0 || rect.top > innerHeight) continue
      const key = text.slice(0, 80)
      found.set(key, found.has(key) ? null : { x: Math.round(rect.x), y: Math.round(rect.y), element })
    }
    return found
  }
  const onScreen = (element) => {
    if (element.closest(pageEnd) || !element.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false
    const rect = element.getBoundingClientRect()
    return rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.top < innerHeight
  }
  const mainShowsSkeleton = () => [...document.querySelectorAll('main#main-content [data-slot="skeleton"]')]
    .some((element) => !stale.has(element) && onScreen(element))
  const mainShowsSomething = () => [...document.querySelectorAll(
    'main#main-content :is([data-slot="skeleton"], p, h1, h2, h3, li, a, button, pre, code, img)',
  )].some(onScreen)
  const sample = () => {
    const now = visibleText()
    const displaced = []
    let compared = 0
    for (const [key, position] of now) {
      if (!position) continue
      const anchor = settled.get(key)
      if (!anchor || (stale.has(anchor.element) && anchor.element !== position.element)) {
        settled.set(key, position)
        candidates.delete(key)
        continue
      }
      compared += 1
      const dx = position.x - anchor.x
      const dy = position.y - anchor.y
      if (Math.abs(dx) > tolerance || Math.abs(dy) > tolerance) {
        displaced.push({ dx, dy, key, position })
        continue
      }
      settled.set(key, position)
      candidates.delete(key)
    }
    if (displaced.length >= RESTYLE_MIN_TEXTS && displaced.length >= compared * RESTYLE_SHARE) {
      for (const { key, position } of displaced) settled.set(key, position)
      candidates.clear()
    } else {
      for (const { dx, dy, key, position } of displaced) {
        const candidate = candidates.get(key)
        const same = candidate && candidate.x === position.x && candidate.y === position.y
        const at = same ? candidate.at : elapsed()
        candidates.set(key, { ...position, at })
        if (elapsed() - at < heldMs) continue
        moves.push({ at, dx, dy, path: describe(position.element), text: key })
        settled.set(key, position)
        candidates.delete(key)
        lastChange = performance.now()
      }
    }
    for (const key of [...settled.keys()]) {
      if (!now.has(key)) settled.delete(key)
    }
    const visible = mainShowsSomething()
    if (mainShowsSkeleton()) pending = true
    if (pending && !visible && !blank) blanks.push({ at: elapsed() })
    if (!visible || candidates.size) lastChange = performance.now()
    blank = pending && !visible
    requestAnimationFrame(sample)
  }

  const begin = () => document.body ? requestAnimationFrame(sample) : setTimeout(begin, 5)
  begin()

  globalThis.__layoutStability = {
    lastChange: () => lastChange,
    report: () => ({ blanks, moves }),
    reset() {
      start = performance.now()
      lastChange = start
      pending = false
      blank = false
      stale = new WeakSet(document.querySelectorAll('body *'))
      candidates = new Map()
      moves = []
      blanks = []
    },
  }
}
