import assert from 'node:assert/strict'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import test from 'node:test'
import { Worker } from 'node:worker_threads'
import { REVIEW_FILE_DIFF_RENDER_BUDGETS } from './review-file-diff-render-contract'
import { diffLineSide, renderedDiffLines } from './review-file-diff-lines'
import { runReviewFileDiffWorker } from './review-file-diff-worker-pool'

test('rendered diff lines follow each side of the prerendered diff, including context and hidden ranges', async () => {
  const oldText = 'a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\n'
  const newText = 'a\nb\nC\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\nnew\no\np\n'
  const worker = new Worker(pathToFileURL(resolve(process.cwd(), 'src/features/review/review-file-diff-render-worker.ts')))
  let result
  try {
    result = await runReviewFileDiffWorker(worker, {
      budget: {
        maxHunks: REVIEW_FILE_DIFF_RENDER_BUDGETS.standard.maxHunks,
        maxOutputBytes: REVIEW_FILE_DIFF_RENDER_BUDGETS.standard.maxOutputBytes,
        maxRenderedLines: REVIEW_FILE_DIFF_RENDER_BUDGETS.standard.maxRenderedLines,
      },
      newText,
      oldText,
      path: '/fixture.txt',
    }, 10_000, undefined, 30_000)
  } finally {
    await worker.terminate()
  }
  assert.equal(result.kind, 'html')
  const lines = renderedDiffLines(result.html)
  assert.deepEqual([...lines.old].sort((a, b) => a - b), [1, 2, 3, 4, 5, 6, 7, 11, 12, 13, 14, 15, 16])
  assert.deepEqual([...lines.new].sort((a, b) => a - b), [1, 2, 3, 4, 5, 6, 7, 11, 12, 13, 14, 15, 16, 17])
})

test('a clicked line number takes its side from the row type, and context rows from their code column', () => {
  assert.equal(diffLineSide('change-deletion', false), 'old')
  assert.equal(diffLineSide('change-addition', true), 'new')
  assert.equal(diffLineSide('context', true), 'old')
  assert.equal(diffLineSide('context', false), 'new')
})
