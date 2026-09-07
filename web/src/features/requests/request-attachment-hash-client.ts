type HashWorker = Pick<Worker, 'onmessage' | 'onerror' | 'postMessage' | 'terminate'>

export async function hashAttachmentFile(blob: Blob, signal: AbortSignal) {
  signal.throwIfAborted()
  const { default: AttachmentHashWorker } = await import('./request-attachment-hash.worker?worker')
  signal.throwIfAborted()
  return runAttachmentHashWorker(blob, signal, new AttachmentHashWorker())
}

export function runAttachmentHashWorker(blob: Blob, signal: AbortSignal, worker: HashWorker) {
  return new Promise<string>((resolve, reject) => {
    function finish(error: Error | null, sha256?: string) {
      signal.removeEventListener('abort', abort)
      worker.onmessage = null
      worker.onerror = null
      worker.terminate()
      if (error) reject(error)
      else resolve(sha256!)
    }
    function abort() {
      finish(new DOMException('Upload cancelled', 'AbortError'))
    }
    worker.onmessage = (event: MessageEvent<{ sha256?: string; error?: string }>) => {
      if (event.data.sha256 && /^[a-f0-9]{64}$/.test(event.data.sha256)) {
        finish(null, event.data.sha256)
      } else {
        finish(new Error(event.data.error ?? 'The file could not be hashed. Try again.'))
      }
    }
    worker.onerror = (event) => {
      event.preventDefault()
      finish(new Error('The file could not be hashed. Try again.'))
    }
    signal.addEventListener('abort', abort, { once: true })
    if (signal.aborted) abort()
    else {
      try { worker.postMessage(blob) }
      catch (error) { finish(error instanceof Error ? error : new Error('The file could not be read.')) }
    }
  })
}
