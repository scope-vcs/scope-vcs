import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import tailwindcss from '@tailwindcss/vite'

const require = createRequire(import.meta.url)
const webPath = (path) => fileURLToPath(new URL(`../../${path}`, import.meta.url))

export const fixturePath = (fixture, path = '.') => webPath(`smoke/fixtures/${fixture}/${path}`)
export const fixtureClerk = { find: '@clerk/tanstack-react-start', replacement: fixturePath('shared', 'clerk.tsx') }

const sharedPage = {
  name: 'scope-shared-fixture-page',
  configureServer(vite) {
    return () => vite.middlewares.use(async (request, response, next) => {
      if (new URL(request.url, 'http://fixture').pathname !== '/index.html') return next()
      try {
        const html = await readFile(fixturePath('shared', 'index.html'), 'utf8')
        response.setHeader('content-type', 'text/html; charset=utf-8')
        response.end(await vite.transformIndexHtml(request.originalUrl ?? request.url, html))
      } catch (error) { next(error) }
    })
  },
}

const reactAliases = ['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
  .map((name) => ({ find: name, replacement: require.resolve(name) }))

function fixtureConfig(fixture, { aliases = [], plugins = [], ssr = false, tailwind = false } = {}) {
  const ownPage = existsSync(fixturePath(fixture, 'index.html'))
  return {
    configFile: false,
    root: fixturePath(fixture),
    plugins: [
      ...(tailwind ? [tailwindcss()] : []),
      ...(ownPage ? [] : [sharedPage]),
      ...plugins,
    ],
    ...(ownPage ? {} : { optimizeDeps: { entries: [fixturePath(fixture, 'entry.tsx')] } }),
    resolve: ssr
      ? { dedupe: ['react', 'react-dom'], alias: [...aliases, { find: '@', replacement: webPath('src') }] }
      : { alias: [...aliases, { find: '@', replacement: webPath('src') }, ...reactAliases] },
    oxc: { jsx: { runtime: 'automatic' } },
  }
}

export async function startFixtureServer(t, fixture, options) {
  const cacheDir = await mkdtemp(join(tmpdir(), `scope-vite-${fixture}-`))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    ...fixtureConfig(fixture, options),
    cacheDir,
    server: { host: '127.0.0.1', port: 0, fs: { allow: [webPath('.')] } },
  })
  t.after(() => server.close())
  await server.listen()
  return server
}

export async function launchBrowser(t) {
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  return browser
}

export async function openFixture(t, fixture, { page: pageOptions = {}, timeout = 10_000, ...options } = {}) {
  const server = await startFixtureServer(t, fixture, options)
  const browser = await launchBrowser(t)
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, ...pageOptions })
  page.setDefaultTimeout(timeout)
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  return { base: server.resolvedUrls.local[0], browser, errors, page }
}
