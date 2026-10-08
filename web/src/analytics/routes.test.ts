import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import test from 'node:test'
import {
  analyticsRouteDecisionForId,
  analyticsRouteForId,
  analyticsRouteForPathname,
  analyticsRouteIds,
} from './routes'

test('route aliases contain no dynamic route values', () => {
  assert.deepEqual(
    analyticsRouteForId('/$owner/$repo/requests/$requestId/changes'),
    {
      name: 'request_changes',
      path: '/repository/request/changes',
    },
  )
  assert.deepEqual(analyticsRouteForId('/$owner/$repo/_code/'), {
    name: 'repository_code',
    path: '/repository/code',
  })
  assert.deepEqual(
    analyticsRouteForId('/$owner/$repo/requests/$requestId/_discussion/'),
    {
      name: 'request',
      path: '/repository/request',
    },
  )
})

test('unknown routes are rejected instead of falling back to a raw path', () => {
  assert.equal(analyticsRouteForId('/adam/private-repo'), null)
  assert.equal(analyticsRouteForId(undefined), null)
})

test('pre-runtime error routes use registered aliases without retaining path values', () => {
  for (const [pathname, name] of [
    ['/', 'home'],
    ['/account', 'account'],
    ['/private-owner/private-repo/', 'repository_code'],
    ['/private-owner/private-repo/requests/private-id', 'request'],
    ['/private-owner/private-repo/requests/private-id/changes', 'request_changes'],
    ['/private-owner/private-repo/runs/private-id', 'repository_run'],
    ['/sign-in/private-flow', 'sign_in'],
  ]) {
    assert.equal(analyticsRouteForPathname(pathname)?.name, name)
  }
  assert.equal(analyticsRouteForPathname('/private-owner/private-repo/private-unknown'), null)
})

test('layout routes are explicitly excluded from page capture', () => {
  for (const routeId of [
    '__root__',
    '/$owner',
    '/$owner/$repo',
    '/$owner/$repo/_code',
    '/$owner/$repo/requests',
    '/$owner/$repo/requests/$requestId',
    '/$owner/$repo/requests/$requestId/_discussion',
    '/$owner/$repo/runs',
  ]) {
    assert.deepEqual(analyticsRouteDecisionForId(routeId), { kind: 'excluded' })
  }
})

test('every generated page or layout route has an analytics decision', () => {
  const routeTree = readFileSync(
    join(process.cwd(), 'src/routeTree.gen.ts'),
    'utf8',
  )
  const fileRoutesById = routeTree.match(
    /export interface FileRoutesById \{(?<routes>[\s\S]*?)\n\}/,
  )?.groups?.routes
  assert.ok(fileRoutesById, 'generated FileRoutesById interface is missing')

  const generatedIds = [...fileRoutesById.matchAll(
    /^\s+(?:'([^']+)'|(__root__)):/gm,
  )]
    .map((match) => match[1] ?? match[2])
    .sort()
  assert.deepEqual(analyticsRouteIds().sort(), generatedIds)
})
