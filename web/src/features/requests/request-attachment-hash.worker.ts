import { createSHA256 } from 'hash-wasm'

const CHUNK_BYTES = 4 * 1024 * 1024

self.onmessage = async (event: MessageEvent<Blob>) => {
  try {
    self.postMessage({ sha256: await sha256Blob(event.data) })
  } catch {
    self.postMessage({ error: 'The file could not be read. Select it again.' })
  }
}

async function sha256Blob(blob: Blob) {
  const hash = await createSHA256()
  for (let offset = 0; offset < blob.size; offset += CHUNK_BYTES) {
    hash.update(new Uint8Array(await blob.slice(offset, offset + CHUNK_BYTES).arrayBuffer()))
  }
  return hash.digest('hex')
}
