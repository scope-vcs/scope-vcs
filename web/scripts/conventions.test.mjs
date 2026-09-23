import assert from 'node:assert/strict'
import { test } from 'node:test'
import { conventionViolations } from './conventions.mjs'

test('route files need a value export named Route', () => {
  const path = 'src/routes/example.tsx'
  assert.deepEqual(conventionViolations(path, 'export const Route = {};'), [])
  assert.deepEqual(conventionViolations(path, 'const route = {}; export { route as Route };'), [])
  assert.equal(conventionViolations(path, 'export type Route = {};').length, 1)
  assert.equal(conventionViolations(path, 'type Route = {}; export type { Route };').length, 1)
  assert.equal(conventionViolations(path, 'const Route = {};').length, 1)
})

test('feature page filenames select the exported function', () => {
  const path = 'src/features/repo-detail/repo-settings-page.tsx'
  assert.deepEqual(conventionViolations(path, 'export function RepoSettingsPage() {}'), [])
  assert.equal(conventionViolations(path, 'function RepoSettingsPage() {}').length, 1)
  assert.equal(conventionViolations(path, 'export function SettingsPage() {}').length, 1)
})
