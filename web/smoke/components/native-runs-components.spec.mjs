import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { fixtureClerk, fixturePath, openFixture } from '../support/fixture-server.mjs'

test('seeded native run navigation makes one read per resource on cold load and reopen', async (t) => {
  const { base, browser, errors, page } = await openFixture(t, 'native-runs', {
    ssr: true,
    timeout: 30_000,
    tailwind: true,
    aliases: [
      fixtureClerk,
      { find: '@/routes/-run-history-actions', replacement: fixturePath('native-runs', 'actions.ts') },
      { find: '@/routes/-repo-settings-actions', replacement: fixturePath('native-runs', 'actions.ts') },
    ],
    plugins: [{
      name: 'native-run-hydration-fixture',
      configureServer(server) {
        server.middlewares.use(async (request, response, next) => {
          if (!new URL(request.url, 'http://fixture').searchParams.has('hydrate')) return next()
          try {
            const { renderFixture } = await server.ssrLoadModule('/server.tsx')
            const { markup, handoff } = await renderFixture(request.url)
            const template = await readFile(fixturePath('shared', 'index.html'), 'utf8')
            const payload = JSON.stringify(handoff).replaceAll('<', '\\u003c')
            const html = template.replace('<div id="root"></div>',
              `<div id="root">${markup}</div><script>window.__nativeRunHandoff=${payload}</script>`)
            response.setHeader('content-type', 'text/html; charset=utf-8')
            response.end(await server.transformIndexHtml(request.url, html))
          } catch (error) { next(error) }
        })
      },
    }],
  })
  for (const [path, resource] of [['/owner/repo/runs', 'history'], ['/owner/repo/runs/run-1', 'detail']]) {
    const hydrated = await browser.newPage()
    hydrated.on('pageerror', (error) => errors.push(error.message))
    hydrated.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
    const url = new URL(`${path}?hydrate=true`, base).href
    const ssr = await hydrated.request.get(url)
    assert.match(await ssr.text(), /<h1[^>]*>/, 'the server must render run markup before hydration')
    await hydrated.goto(url)
    await hydrated.waitForFunction(() => [...document.querySelectorAll('h1')].some((element) =>
      Object.keys(element).some((key) => key.startsWith('__reactProps$'))))
    await hydrated.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)))
    assert.equal(await hydrated.evaluate((key) => window.__nativeRunHandoff.loads[key], resource), 1)
    assert.equal(await hydrated.evaluate((key) => window.loads[key], resource), 1,
      'hydration must retain the server resource without another metadata read')
    await hydrated.close()
  }
  await page.goto(new URL('/owner/repo/runs', base).href)
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)))
  assert.equal(await page.evaluate(() => window.loads.history), 1, 'seeded cold history load must not read again on mount')
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByRole('heading', { name: /Runs.*tests/ }).waitFor()
  await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)))
  assert.equal(await page.evaluate(() => window.loads.detail), 1, 'seeded cold detail load must not read again on mount')
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByRole('heading', { name: /Runs.*tests/ }).waitFor()
  assert.deepEqual(await page.evaluate(() => ({ history: window.loads.history, detail: window.loads.detail })), { history: 1, detail: 1 })
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-${width}.png` })
  }
  await page.evaluate(() => window.emitChange('Connected'))
  assert.deepEqual(await page.evaluate(() => ({ history: window.loads.history, detail: window.loads.detail })), { history: 1, detail: 1 })
  await page.evaluate(() => window.emitChange('Lagged'))
  await page.waitForFunction(() => window.loads.detail === 2)
  await page.evaluate(() => { window.holdLoads(); window.completeRun(); window.emitChange({ RunChanged: { run_id: 'run-1', change: 'StatusChanged' } }) })
  await page.waitForFunction(() => window.loads.detail === 3)
  assert.equal(await page.getByRole('heading', { name: /Runs.*tests/ }).count(), 1)
  await page.evaluate(() => window.releaseLoads())
  await page.getByRole('button', { name: 'Run again' }).waitFor()
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  const beforeCatalogRefresh = await page.evaluate(() => ({ ...window.loads }))
  await page.evaluate(() => {
    window.holdLoads()
    window.emitChange({ RepositoryChanged: { reason: 'push' } })
  })
  await page.waitForFunction((reads) => window.loads.history === reads + 1, beforeCatalogRefresh.history)
  assert.equal(await page.evaluate(() => window.loads.workflows), beforeCatalogRefresh.workflows + 1,
    'repository refresh must reuse the page catalog read')
  await page.evaluate(() => window.releaseLoads())
  await page.evaluate(() => window.holdLoads())
  await page.getByRole('button', { name: 'Load older runs' }).click()
  const paginationCatalogReads = await page.evaluate(() => window.loads.workflows)
  await page.evaluate(() => window.emitChange({ RepositoryChanged: { reason: 'push' } }))
  await page.waitForFunction((reads) => window.loads.workflows === reads + 1, paginationCatalogReads)
  await page.evaluate(() => window.releaseLoads())
  await page.getByText('older tests', { exact: true }).waitFor()
  await page.getByRole('link', { name: 'Away', exact: true }).click()
  await page.evaluate(() => window.emitChange({ RunChanged: { run_id: 'run-1', change: 'Created' } }))
  await page.evaluate(() => window.holdLoads())
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByText('older tests', { exact: true }).waitFor()
  await page.evaluate(() => window.releaseLoads())
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByText('retained build output', { exact: false }).waitFor()
  const completedReads = await page.evaluate(() => window.loads.detail)
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByText('retained build output', { exact: false }).waitFor()
  assert.equal(await page.evaluate(() => window.loads.detail), completedReads)
  await page.evaluate(() => { window.holdLoads(); window.setPermitted(false); window.setActor('Public') })
  await page.getByText('Loading run details', { exact: true }).waitFor()
  assert.equal(await page.getByText('retained build output', { exact: false }).count(), 0)
  assert.equal(await page.getByRole('heading', { name: /Runs.*tests/ }).count(), 0)
  await page.evaluate(() => window.releaseLoads())
  await page.getByText('Run denied', { exact: true }).waitFor()
  await page.evaluate(() => { window.holdLoads(); window.setViewer('other') })
  await page.getByText('Loading run details', { exact: true }).waitFor()
  assert.equal(await page.getByText('retained build output', { exact: false }).count(), 0)
  await page.evaluate(() => window.releaseLoads())
  await page.getByText('Run denied', { exact: true }).waitFor()

  const cold = await browser.newPage()
  cold.on('pageerror', (error) => errors.push(error.message))
  await cold.goto(new URL('/owner/repo/runs?client=true', base).href)
  await cold.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  assert.equal(await cold.evaluate(() => window.loads.history), 1)
  await cold.getByRole('link', { name: 'Detail', exact: true }).click()
  await cold.getByRole('heading', { name: /Runs.*tests/ }).waitFor()
  assert.equal(await cold.evaluate(() => window.loads.detail), 1)
  await cold.getByRole('button', { name: 'Cancel', exact: true }).click()
  await cold.getByRole('button', { name: 'Run again' }).waitFor()
  assert.equal(await cold.evaluate(() => window.loads.detail), 2)
  assert.deepEqual(errors, [])
})
