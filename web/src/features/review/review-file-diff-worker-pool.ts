import type { Worker } from 'node:worker_threads'
import type {
  ReviewFileDiffWorkerInput,
  ReviewFileDiffWorkerResult,
} from './review-file-diff-render-contract'
import { ReviewDiffTransientError } from './review-file-diff-renderer'

export function createReviewFileDiffWorkerPool(
  createWorker: () => Worker,
  maxWorkers: number,
  startupDeadlineMs?: number,
) {
  const workers = new Map<Worker, boolean>()

  async function discard(worker: Worker) {
    try {
      await worker.terminate()
    } finally {
      workers.delete(worker)
    }
  }

  return async function render(
    input: ReviewFileDiffWorkerInput,
    deadlineMs: number,
    signal?: AbortSignal,
  ): Promise<ReviewFileDiffWorkerResult> {
    signal?.throwIfAborted()
    let worker = [...workers].find(([, busy]) => !busy)?.[0]
    const starting = !worker
    if (!worker) {
      if (workers.size >= maxWorkers) throw new ReviewDiffTransientError('busy')
      worker = createWorker()
      workers.set(worker, false)
      const created = worker
      created.on('error', () => { void discard(created).catch(() => {}) })
      created.on('exit', () => workers.delete(created))
    }
    workers.set(worker, true)
    worker.ref()
    try {
      const result = await runReviewFileDiffWorker(
        worker, input, deadlineMs, signal, starting ? startupDeadlineMs : undefined,
      )
      if (result.kind === 'error') await discard(worker).catch(() => {})
      return result
    } catch (error) {
      await discard(worker).catch(() => {})
      throw error
    } finally {
      if (workers.has(worker)) {
        workers.set(worker, false)
        worker.unref()
      }
    }
  }
}

export function runReviewFileDiffWorker(
  worker: Pick<Worker, 'once' | 'off' | 'postMessage'>,
  input: ReviewFileDiffWorkerInput,
  deadlineMs: number,
  signal?: AbortSignal,
  startupDeadlineMs = deadlineMs,
): Promise<ReviewFileDiffWorkerResult> {
  signal?.throwIfAborted()
  return new Promise((resolve, reject) => {
    let settled = false
    const finish = (result?: ReviewFileDiffWorkerResult, error?: unknown) => {
      if (settled) return
      settled = true
      clearTimeout(deadline)
      worker.off('message', onMessage)
      worker.off('error', onError)
      worker.off('exit', onExit)
      signal?.removeEventListener('abort', onAbort)
      if (result) resolve(result)
      else reject(error)
    }
    const onMessage = (result: ReviewFileDiffWorkerResult | { kind: 'ready' }) => {
      if (result.kind === 'ready') {
        clearTimeout(deadline)
        deadline = setTimeout(onDeadline, deadlineMs)
        worker.once('message', onMessage)
      } else {
        finish(result)
      }
    }
    const onError = () => finish(undefined, new Error('This file diff could not be rendered.'))
    const onExit = () => finish(undefined, new Error('The diff renderer exited before returning a result.'))
    const onAbort = () => finish(undefined, signal?.reason)
    const onDeadline = () => finish(undefined, new ReviewDiffTransientError('deadline'))
    let deadline = setTimeout(onDeadline, startupDeadlineMs)
    worker.once('message', onMessage)
    worker.once('error', onError)
    worker.once('exit', onExit)
    signal?.addEventListener('abort', onAbort, { once: true })
    try {
      worker.postMessage(input)
    } catch (error) {
      finish(undefined, error)
    }
  })
}
