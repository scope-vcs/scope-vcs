import assert from 'node:assert/strict'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import test from 'node:test'
import { Worker } from 'node:worker_threads'
import {
  codeTextMetrics,
  type CodeRenderDiffInput,
  type CodeRenderWorkerResult,
} from './code-render-contract'
import { runCodeRenderWorker } from './code-render-worker-pool'

const COLD_MODULE_COMPILATION_DEADLINE_MS = 30_000

test('counts UTF-8 bytes, lines, and the longest line exactly', () => {
  assert.deepEqual(codeTextMetrics('a😀\nβ'), {
    bytes: 8,
    lines: 2,
    maxLineBytes: 5,
  })
  assert.deepEqual(codeTextMetrics(''), {
    bytes: 0,
    lines: 0,
    maxLineBytes: 0,
  })
})

test('the source worker bounds diffs and files and picks each grammar from the file name', async (t) => {
  const worker = new Worker(pathToFileURL(resolve(process.cwd(), 'src/features/code-render/code-render-worker.ts')))
  t.after(() => worker.terminate())
  let startupDeadlineMs = COLD_MODULE_COMPILATION_DEADLINE_MS
  const render = async (input: Parameters<typeof runCodeRenderWorker>[1]) => {
    const result = await runCodeRenderWorker(worker, input, 10_000, undefined, startupDeadlineMs)
    startupDeadlineMs = 10_000
    return result
  }
  const diff = (
    oldText: string,
    newText: string,
    budget: Partial<CodeRenderDiffInput['budget']> = {},
    path = '/fixture.ts',
  ): CodeRenderDiffInput => ({
    budget: { maxHunks: 64, maxOutputBytes: 256 * 1024, maxRenderedLines: 800, ...budget },
    kind: 'diff',
    newText,
    oldText,
    path,
  })

  assert.deepEqual(await render(diff('same\n', 'same\n')), { kind: 'empty' })
  assert.deepEqual(await render(diff('a\n', 'b\n', { maxHunks: 0 })), { kind: 'omitted', reason: 'hunks' })
  assert.deepEqual(await render(diff('a\n', 'b\n', { maxRenderedLines: 1 })), { kind: 'omitted', reason: 'lines' })
  assert.deepEqual(await render(diff('a\n', 'b\n', { maxOutputBytes: 100 })), { kind: 'omitted', reason: 'output' })
  const thousand = (name: string) => Array.from(
    { length: 1_000 },
    (_, index) => `export const ${name}${index} = "aaaaaaaaaaaa"\n`,
  ).join('')
  assert.deepEqual(
    await render(diff(thousand('before'), thousand('after'))),
    { kind: 'omitted', reason: 'lines' },
  )

  const toml = '[package]\nname = "fixture"\nversion = "0.1.0"\n'
  assert.deepEqual(
    await render({ budget: { maxOutputBytes: 100 }, kind: 'file', path: 'crates/fixture/Cargo.toml', text: toml }),
    { kind: 'omitted', reason: 'output' },
  )
  assert.ok(tokenColors(await render({
    budget: { maxOutputBytes: 256 * 1024 },
    kind: 'file',
    path: 'crates/fixture/Cargo.toml',
    text: toml,
  })) > 1)
  assert.ok(tokenColors(await render(diff(
    'FROM node:24\nRUN echo old\n',
    'FROM node:24\nRUN echo new\n',
    {},
    'deploy/Dockerfile',
  ))) > 1)
})

function tokenColors(result: CodeRenderWorkerResult) {
  assert.equal(result.kind, 'html')
  return new Set(result.html.match(/--diffs-token-light:#\w+/g)).size
}
