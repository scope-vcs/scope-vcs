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
const DIVIDER_TOLERANCE_PX = 4
const DIVIDER_MATCH_RATIO = 0.8
const EDGE_TOLERANCE_PX = 2
const MAINTAINER_ONLY_SECTIONS = ['Runs', 'Settings']

const storageState = process.env.SCOPE_SMOKE_STORAGE_STATE

const SCENARIOS = [
  { name: 'profile', from: repoPath, to: `/${owner}` },
  { name: 'code from profile', from: `/${owner}`, to: repoPath },
  { name: 'code from update', from: `${repoPath}/updates/dev-public-1`, to: repoPath },
  { name: 'update from profile', from: `/${owner}`, to: `${repoPath}/updates/dev-public-1` },
  { name: 'update from code', from: repoPath, to: `${repoPath}/updates/dev-public-1` },
  { name: 'requests from profile', from: `/${owner}`, to: `${requestRepoPath}/requests` },
  { name: 'requests from code', from: requestRepoPath, to: `${requestRepoPath}/requests` },
  ...storageState ? [
    { name: 'runs', from: repoPath, to: `${repoPath}/runs`, signedIn: true },
    { name: 'run detail', from: `${repoPath}/runs`, to: firstRunLink, signedIn: true },
    { name: 'settings', from: `/${owner}`, to: `${repoPath}/settings`, signedIn: true },
    { name: 'account', from: `/${owner}`, to: '/account', signedIn: true },
    { name: 'request detail', from: `${requestRepoPath}/requests`, to: firstRequestLink, signedIn: true },
  ] : [],
]

const KNOWN_FAILURES = new Set([
  'mobile update from code: dividers',
  'mobile update from profile: dividers',
])

for (const [viewportName, viewport] of Object.entries(VIEWPORTS)) {
  for (const scenario of SCENARIOS) {
    const id = `${viewportName} ${scenario.name}`
    test(`skeleton layout: ${id}`, async () => {
      await withPage(scenario.from, async (page) => {
        const steps = await captureNavigation(page, scenario.to)
        assert(steps.pending.length > 0, `${id} never showed a skeleton`)
        const problems = Object.entries(compareSteps(steps)).flatMap(([check, failure]) => {
          const key = `${id}: ${check}`
          if (KNOWN_FAILURES.has(key)) return failure ? [] : [`${key} passes now; remove it from KNOWN_FAILURES`]
          return failure ? [`${key}: ${failure}`] : []
        })
        assert.deepEqual(problems, [])
      }, { viewport, ...scenario.signedIn ? { storageState } : {} })
    })
  }
}

async function firstRunLink(page) {
  return page.evaluate(() => [...document.querySelectorAll('a[href*="/runs/"]')]
    .map((link) => link.getAttribute('href'))
    .find((href) => !href.includes('/workflows/')))
}

async function firstRequestLink(page) {
  const link = page.locator('.request-workspace-rows a[href*="/requests/"]').first()
  await link.waitFor()
  return link.getAttribute('href')
}

async function captureNavigation(page, target) {
  await waitForClientHydration(page.locator('header.application-topbar a').first())
  await waitForIdle(page)
  const href = typeof target === 'function' ? await target(page) : target
  assert(href, 'navigation target was not found on the start page')

  let holding = true
  let held = []
  await page.route('**/_serverFn/**', (route) => {
    if (holding) held.push(route)
    else void route.continue()
  })
  await page.evaluate((next) => { void globalThis.__TSR_ROUTER__.navigate({ href: next }) }, href)
  for (let waited = 0; held.length === 0 && waited < 10_000; waited += 100) {
    await page.waitForTimeout(100)
  }

  const pending = []
  for (let step = 0; step < 8; step += 1) {
    await page.waitForTimeout(700)
    const layout = await page.evaluate(measureLayout)
    if (layout.skeletons > 0) pending.push(layout)
    if (held.length === 0) break
    const batch = held
    held = []
    await Promise.all(batch.map(release))
  }
  holding = false
  await Promise.all(held.map(release))
  await waitForIdle(page)
  await page.waitForFunction(
    () => document.querySelectorAll('main [data-slot="skeleton"]').length === 0,
    undefined,
    { timeout: 30_000 },
  )
  let loaded
  for (let waited = 0; waited < 10_000; waited += 200) {
    await page.waitForTimeout(200)
    loaded = await page.evaluate(measureLayout)
    if (Number.isFinite(loaded.left)) break
  }
  return { pending, loaded }
}

async function release(route) {
  try {
    await route.fulfill({ response: await route.fetch({ timeout: 15_000 }) })
  } catch {}
}

async function waitForIdle(page) {
  await page.waitForFunction(() => globalThis.__TSR_ROUTER__?.state.status === 'idle')
}

function compareSteps({ pending, loaded }) {
  const failures = { topbar: null, 'content edge': null, dividers: null }
  for (const [index, step] of pending.entries()) {
    const at = `step ${index + 1}`
    if (step.topbar.height !== loaded.topbar.height) {
      failures.topbar ??= `${at} topbar is ${step.topbar.height}px, loaded is ${loaded.topbar.height}px`
    } else {
      const extra = step.topbar.sections.filter((label) => !loaded.topbar.sections.includes(label))
      const missing = loaded.topbar.sections.filter((label) =>
        !step.topbar.sections.includes(label) && !MAINTAINER_ONLY_SECTIONS.includes(label))
      if (extra.length) failures.topbar ??= `${at} shows sections the page lacks: ${extra.join(', ')}`
      if (missing.length) failures.topbar ??= `${at} is missing sections: ${missing.join(', ')}`
    }
    if (Math.abs(step.left - loaded.left) > EDGE_TOLERANCE_PX) {
      failures['content edge'] ??= `${at} content starts at x=${step.left}, loaded at x=${loaded.left}`
    }
    const expected = loaded.dividers
    const matched = expected.filter((y) => near(step.dividers, y)).length
    const drawn = step.dividers.filter((y) => y <= Math.max(0, ...loaded.dividers) + DIVIDER_TOLERANCE_PX)
    const kept = drawn.filter((y) => near(loaded.dividers, y)).length
    if (
      matched < expected.length * DIVIDER_MATCH_RATIO ||
      kept < drawn.length * DIVIDER_MATCH_RATIO
    ) {
      failures.dividers ??= `${at} dividers [${step.dividers}] vs loaded [${loaded.dividers}]`
    }
  }
  return failures
}

function near(values, y) {
  return values.some((value) => Math.abs(value - y) <= DIVIDER_TOLERANCE_PX)
}

function measureLayout() {
  const visible = (element) => {
    const rect = element.getBoundingClientRect()
    return rect.width > 0 && rect.height > 0 && getComputedStyle(element).visibility !== 'hidden'
  }
  const header = [...document.querySelectorAll('header.application-topbar')].find(visible)
  const main = [...document.querySelectorAll('main#main-content')].find(visible)
  const sections = header
    ? [...header.querySelectorAll('nav[aria-label="Primary"] a')]
        .filter(visible)
        .map((link) => link.firstChild?.textContent?.trim() ?? '')
    : []
  let left = Infinity
  const dividers = new Set()
  const mainRect = main?.getBoundingClientRect()
  for (const element of main?.querySelectorAll('*') ?? []) {
    if (!visible(element) || element.closest('.sr-only, .animate-spin')) continue
    const rect = element.getBoundingClientRect()
    if (rect.bottom < mainRect.top || rect.top > innerHeight) continue
    if (element.matches('[data-slot="skeleton"], svg, img, input, textarea, select')) {
      left = Math.min(left, rect.left)
    }
    for (const node of element.childNodes) {
      if (node.nodeType !== Node.TEXT_NODE || !node.textContent.trim()) continue
      const range = document.createRange()
      range.selectNodeContents(node)
      left = Math.min(left, range.getBoundingClientRect().left)
    }
    if (rect.width < mainRect.width * 0.25) continue
    const style = getComputedStyle(element)
    const edge = (side) => parseFloat(style[`border${side}Width`]) > 0 && style[`border${side}Style`] !== 'none'
    if (edge('Left') || edge('Right')) continue
    if (edge('Top')) dividers.add(Math.round(rect.top))
    if (edge('Bottom')) dividers.add(Math.round(rect.bottom))
  }
  return {
    dividers: [...dividers].filter((y) => y > (mainRect?.top ?? 0) && y < innerHeight).sort((a, b) => a - b),
    left: Math.round(left),
    skeletons: main ? [...main.querySelectorAll('[data-slot="skeleton"]')].filter(visible).length : 0,
    topbar: {
      height: Math.round(header?.getBoundingClientRect().height ?? 0),
      sections,
    },
  }
}
