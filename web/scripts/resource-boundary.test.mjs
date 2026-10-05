import assert from 'node:assert/strict'
import { test } from 'node:test'
import { resourceBoundaryViolations } from './resource-boundary.mjs'

const check = (source) => resourceBoundaryViolations('src/features/example.tsx', source)
test('blocks direct, aliased, and locally wrapped server reads in effects', () => {
  for (const source of [
    `import {useEffect} from 'react'; function View(){ useEffect(() => { fetch('/data').then(setData) }, []) }`,
    `import {useEffect as effect} from 'react'; import {loadThings as read} from '@/routes/-things'; function View(){ effect(() => { read().then(setData) }, []) }`,
    `import {useEffect,useCallback} from 'react'; import {loadThings} from '@/api/things'; function View(){ const run = useCallback(() => loadThings(), []); useEffect(() => { void run() }, [run]) }`,
    `import {useEffect} from 'react'; import * as api from '@/api/things'; function View(){ useEffect(() => { api.loadThings() }, []) }`,
  ]) assert.equal(check(source).length, 1)
})
test('allows resource declarations and deliberate event handlers', () => {
  assert.deepEqual(check(`import {useEffect} from 'react'; import {loadThings} from '@/routes/-things'; function View(){ const resource = useCachedResource({load: loadThings}); useEffect(() => subscribe(resource.retry), []); return <button onClick={() => loadThings()}/> }`), [])
})
test('handles local recursive helpers without hanging', () => {
  assert.deepEqual(check(`import {useEffect} from 'react'; function View(){ const tick=()=>tick(); useEffect(tick, []) }`), [])
})

const checkRoute = (source) => resourceBoundaryViolations('src/routes/example.tsx', source)
test('blocks any api import called inside a route effect, whatever it is named', () => {
  for (const source of [
    `import {useEffect} from 'react'; import {refreshThings} from '@/api/things'; function Page(){ useEffect(() => { void refreshThings() }, []) }`,
    `import {useEffect} from 'react'; import {getThings} from '../api/things'; function Page(){ useEffect(() => { getThings().then(setData) }, []) }`,
    `import {useEffect} from 'react'; import * as api from '@/api/things'; function Page(){ useEffect(() => { api.refreshThings() }, []) }`,
    `import {useEffect} from 'react'; import client from '@/api/client'; function Page(){ useEffect(() => { client() }, []) }`,
  ]) assert.equal(checkRoute(source).length, 1)
})
test('leaves type-only api imports and non-loader route imports alone', () => {
  for (const source of [
    `import {useEffect} from 'react'; import type {formatThing} from '@/api/things'; function Page(){ const formatThing = () => null; useEffect(() => { formatThing() }, []) }`,
    `import {useEffect} from 'react'; import {type Things, formatThing} from '@/routes/-things'; function Page(){ useEffect(() => { formatThing() }, []) }`,
  ]) assert.deepEqual(checkRoute(source), [])
})
test('allows route effects that call resource owner behavior and navigate', () => {
  assert.deepEqual(checkRoute(`import {useEffect} from 'react'; import {retainThings} from '@/features/things/things-resource'; function Page(){ useEffect(() => { retainThings(identity, value); void navigate({to: '/things'}) }, []) }`), [])
})

test('rejects route writes to imported resource owners', () => {
  for (const method of ['write', 'seed', 'writeIfUnchanged']) {
    const source = `import { cliSessionsResource as sessions } from '@/features/account/cli-sessions-resource'; function AccountRoute() { sessions.${method}(identity, loaded.sessions) }`
    assert.match(checkRoute(source)[0], /cli-sessions-resource owner function/)
  }
  assert.match(checkRoute(`import * as account from '@/features/account/cli-sessions-resource'; account['cliSessionsResource']['write'](identity, loaded.sessions)`)[0], /cli-sessions-resource owner function/)
  assert.match(checkRoute(`import { alias } from '@/features/account/cli-sessions-resource'; alias['write'](identity, loaded.sessions)`)[0], /cli-sessions-resource owner function/)
})

test('rejects the historical account route publication after loader revalidation', () => {
  const source = `import { useEffect } from 'react'
import { cliSessionsResource } from '@/features/account/cli-sessions-resource'
function AccountRoute() {
  useEffect(() => {
    if (loaded.viewerId !== userId || !identity) return
    cliSessionsResource.write(identity, loaded.sessions)
  }, [identity, loaded, userId])
}`
  assert.match(checkRoute(source)[0], /route publishes cliSessionsResource directly/)
})

test('allows routes to call owner behavior and unrelated object writers', () => {
  assert.deepEqual(checkRoute(`import { retainRevokedCliSession } from '@/features/account/cli-sessions-resource'; function AccountRoute() { retainRevokedCliSession(viewerId, sessionId); editor.write(text) }`), [])
})
