import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubConnectionResponse } from '@/api/types.generated'
import { runsCiOffer } from './runs-ci-offer-model'

const notConnected: GitHubConnectionResponse = {
  configured: true,
  connection: null,
  required_checks: [],
  can_confirm_public: true,
  setup_check: null,
  run_import_count: 50,
  run_import: null,
}

const offer = (patch: Partial<Parameters<typeof runsCiOffer>[0]> = {}) => runsCiOffer({
  configured: true,
  github: notConnected,
  hasWorkflows: false,
  maintainer: true,
  ...patch,
})

test('a maintainer of a repository without runs or GitHub is offered to connect it, without the button until the connection loads', () => {
  assert.deepEqual(offer(), { kind: 'connect', canConnect: true })
  assert.deepEqual(offer({ github: null }), { kind: 'connect', canConnect: false })
})

test('other members read where runs come from but cannot connect', () => {
  assert.deepEqual(offer({ maintainer: false, github: null }), { kind: 'connect', canConnect: false })
})

test('a server without GitHub promises nothing', () => {
  assert.deepEqual(offer({ configured: false }), { kind: 'none' })
  assert.deepEqual(offer({ configured: false, maintainer: false, github: null }), { kind: 'none' })
  assert.deepEqual(offer({ github: { ...notConnected, configured: false } }), { kind: 'none' })
})

test('a repository with workflows of its own or a GitHub link keeps its empty state', () => {
  assert.deepEqual(offer({ hasWorkflows: true }), { kind: 'none' })
  const connection = {
    github_full_name: 'octo/demo',
    github_url: 'https://github.com/octo/demo',
    connected_by: null,
    connected_at_unix: 1,
    disconnected: null,
    public_on_github: false,
    public_confirmed: true,
  }
  assert.deepEqual(offer({ github: { ...notConnected, connection } }), { kind: 'none' })
})
