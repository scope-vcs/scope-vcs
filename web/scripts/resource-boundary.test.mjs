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
