import type { Worker } from 'node:worker_threads'
import type {
  CodeRenderWorkerInput,
  CodeRenderWorkerResult,
} from './code-render-contract'

type CodeRenderTransientFailure = 'busy' | 'deadline'

export class CodeRenderTransientError extends Error {
  constructor(readonly failure: CodeRenderTransientFailure) {
    super(failure === 'busy'
      ? 'The diff renderer is busy. Try again.'
      : 'Diff rendering took too long. Try again.')
    this.name = 'CodeRenderTransientError'
  }
}

export function createCodeRenderWorkerPool(
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
    input: CodeRenderWorkerInput,
    deadlineMs: number,
    signal?: AbortSignal,
  ): Promise<CodeRenderWorkerResult> {
    signal?.throwIfAborted()
    let worker = [...workers].find(([, busy]) => !busy)?.[0]
    const starting = !worker
    if (!worker) {
      if (workers.size >= maxWorkers) throw new CodeRenderTransientError('busy')
      worker = createWorker()
      workers.set(worker, false)
      const created = worker
      created.on('error', () => { void discard(created).catch(() => {}) })
      created.on('exit', () => workers.delete(created))
    }
    workers.set(worker, true)
    worker.ref()
    try {
      const result = await runCodeRenderWorker(
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

export function runCodeRenderWorker(
  worker: Pick<Worker, 'once' | 'off' | 'postMessage'>,
  input: CodeRenderWorkerInput,
  deadlineMs: number,
  signal?: AbortSignal,
  startupDeadlineMs = deadlineMs,
): Promise<CodeRenderWorkerResult> {
  signal?.throwIfAborted()
  return new Promise((resolve, reject) => {
    let settled = false
    const settle = (outcome: () => void) => {
      if (settled) return
      settled = true
      clearTimeout(deadline)
      worker.off('message', onMessage)
      worker.off('error', onError)
      worker.off('exit', onExit)
      signal?.removeEventListener('abort', onAbort)
      outcome()
    }
    const fail = (error: Error) => settle(() => reject(error))
    const onMessage = (result: CodeRenderWorkerResult | { kind: 'ready' }) => {
      if (result.kind === 'ready') {
        clearTimeout(deadline)
        deadline = setTimeout(onDeadline, deadlineMs)
        worker.once('message', onMessage)
      } else {
        settle(() => resolve(result))
      }
    }
    const onError = () => fail(new Error('This file diff could not be rendered.'))
    const onExit = () => fail(new Error('The diff renderer exited before returning a result.'))
    const onAbort = () => {
      const reason: unknown = signal?.reason
      fail(reason instanceof Error ? reason : new DOMException('The diff render was aborted.', 'AbortError'))
    }
    const onDeadline = () => fail(new CodeRenderTransientError('deadline'))
    let deadline = setTimeout(onDeadline, startupDeadlineMs)
    worker.once('message', onMessage)
    worker.once('error', onError)
    worker.once('exit', onExit)
    signal?.addEventListener('abort', onAbort, { once: true })
    try {
      worker.postMessage(input)
    } catch (error) {
      fail(error instanceof Error ? error : new Error('The diff renderer could not receive the file.'))
    }
  })
}
