import assert from 'node:assert/strict'
import test from 'node:test'
import { cloneCommands } from './clone-command'

const base = { owner: 'acme', repo: 'demo', readerView: 'agent' }

test('anonymous readers clone the selected view over HTTPS', () => {
  assert.deepEqual(
    cloneCommands({ ...base, actor: 'Public', readerView: 'public', remoteUrl: 'https://scope.test/git/public/acme/demo', view: 'public' })
      .map((command) => command.value),
    ['git clone https://scope.test/git/public/acme/demo'],
  )
})

test('signed-in readers get the CLI command for their own view and the view remote', () => {
  assert.deepEqual(
    cloneCommands({ ...base, actor: 'Member', remoteUrl: 'https://scope.test/git/agent/acme/demo', view: 'agent' })
      .map((command) => command.value),
    ['scope clone acme/demo', 'https://scope.test/git/agent/acme/demo'],
  )
})

test('selecting a narrower view adds it to the CLI command and the remote', () => {
  assert.deepEqual(
    cloneCommands({ ...base, actor: 'Owner', readerView: 'private', remoteUrl: 'https://scope.test/git/agent/acme/demo', view: 'agent' })
      .map((command) => command.value),
    ['scope clone acme/demo --view agent', 'https://scope.test/git/agent/acme/demo'],
  )
})
