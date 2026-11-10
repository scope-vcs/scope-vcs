import assert from 'node:assert/strict'
import test from 'node:test'
import { isDocument } from './check-docs.mjs'

test('reports documentation files anywhere in the repository', () => {
  for (const file of [
    'docs/architecture.md',
    'docs/fixtures/architecture.md',
    'docs/verification/stack.png',
    'deploy/aws/OPERATIONS.md',
    'bench/README.md',
    'web/README',
    'crates/scope-git/NOTES.MD',
    'api/guide.rst',
    'web/src/features/requests/overview.mdx',
  ]) assert.equal(isDocument(file), true, file)
})

test('leaves code, agent instructions, the homepage, legal text, and fixtures alone', () => {
  for (const file of [
    'api/src/main.rs',
    'web/src/styles.css',
    'api/src/docs/render.rs',
    'AGENTS.md',
    'CLAUDE.md',
    '.agents/skills/scope-tests/SKILL.md',
    'README.md',
    'README.html',
    'legal/privacy-policy.md',
    '.github/SECURITY.md',
    'media-worker/CODEC-NOTICE.md',
    'crates/scope-dependency-analyzer/tests/fixtures/mixed/README.md',
  ]) assert.equal(isDocument(file), false, file)
})

test('agent instructions and the homepage are permitted only at the root', () => {
  assert.equal(isDocument('web/AGENTS.md'), true)
  assert.equal(isDocument('cli/README.md'), true)
})
