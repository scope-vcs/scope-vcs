import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import type { RepoFileContentResponse } from '../../api/types.generated'
import type { CodeRenderFileInput, CodeRenderWorkerResult } from '../code-render/code-render-contract'
import { CodeRenderTransientError } from '../code-render/code-render-worker-pool'
import { createRepoFileHighlighter, REPO_FILE_HIGHLIGHT_LIMITS } from './repo-file-highlight'

function textFile(path: string, text: string): RepoFileContentResponse {
  return { content: { kind: 'text', text }, label: 'public', oid: 'oid', path, size_bytes: text.length }
}

function fakeRender(result: () => Promise<CodeRenderWorkerResult>) {
  return mock.fn((_input: CodeRenderFileInput, _deadlineMs: number, _signal?: AbortSignal) => result())
}

test('highlights source files within the limits and leaves markdown, binaries, and oversized files plain', async () => {
  const isolatedRender = fakeRender(async () => ({ html: '<pre>highlighted</pre>', kind: 'html' }))
  const highlight = createRepoFileHighlighter({ isolatedRender })
  const plain = [
    textFile('docs/guide.md', '# Guide\n'),
    { content: { kind: 'binary', oid: 'oid', size_bytes: 4 }, label: 'public', oid: 'oid', path: 'logo.png', size_bytes: 4 },
    textFile('big.rs', 'x'.repeat(REPO_FILE_HIGHLIGHT_LIMITS.maxInputBytes + 1)),
    textFile('long.rs', '\n'.repeat(REPO_FILE_HIGHLIGHT_LIMITS.maxInputLines + 1)),
  ] satisfies RepoFileContentResponse[]
  for (const file of plain) {
    assert.deepEqual(await highlight(file), { ...file, highlightedHtml: null })
  }
  assert.equal(isolatedRender.mock.callCount(), 0)

  const source = textFile('src/page.html', '<h1>Scope</h1>\n')
  assert.deepEqual(await highlight(source), { ...source, highlightedHtml: '<pre>highlighted</pre>' })
  assert.deepEqual(isolatedRender.mock.calls[0]?.arguments.slice(0, 2), [{
    budget: { maxOutputBytes: REPO_FILE_HIGHLIGHT_LIMITS.maxOutputBytes },
    kind: 'file',
    path: 'src/page.html',
    text: '<h1>Scope</h1>\n',
  }, REPO_FILE_HIGHLIGHT_LIMITS.deadlineMs])
})

test('a busy, slow, failed, or oversized render still loads the file as plain text, but cancellation propagates', async (t) => {
  t.mock.method(console, 'warn', () => {})
  const source = textFile('src/lib.rs', 'fn main() {}\n')
  const outcomes: Array<() => Promise<CodeRenderWorkerResult>> = [
    async () => { throw new CodeRenderTransientError('deadline') },
    async () => { throw new Error('The diff renderer exited before returning a result.') },
    async () => ({ kind: 'error' }),
    async () => ({ kind: 'omitted', reason: 'output' }),
  ]
  for (const outcome of outcomes) {
    const highlight = createRepoFileHighlighter({ isolatedRender: fakeRender(outcome) })
    assert.deepEqual(await highlight(source), { ...source, highlightedHtml: null })
  }

  const isolatedRender = fakeRender(async () => ({ html: 'unused', kind: 'html' }))
  const state = { active: REPO_FILE_HIGHLIGHT_LIMITS.maxConcurrentRenders }
  const busy = createRepoFileHighlighter({ isolatedRender, state })
  assert.deepEqual(await busy(source), { ...source, highlightedHtml: null })
  assert.equal(isolatedRender.mock.callCount(), 0)

  const controller = new AbortController()
  const canceled = createRepoFileHighlighter({
    isolatedRender: fakeRender(async () => {
      controller.abort()
      throw new DOMException('The diff render was aborted.', 'AbortError')
    }),
  })
  await assert.rejects(canceled(source, controller.signal), { name: 'AbortError' })
})
