import { sha256Blob } from './request-attachment-hash'

self.onmessage = async (event: MessageEvent<Blob>) => {
  try {
    self.postMessage({ sha256: await sha256Blob(event.data) })
  } catch {
    self.postMessage({ error: 'The file could not be read. Select it again.' })
  }
}
