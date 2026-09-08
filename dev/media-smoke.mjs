#!/usr/bin/env node

import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, open, stat, writeFile } from 'node:fs/promises'
import { basename, dirname } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const args = process.argv.slice(2)
const value = (name, fallback = '') => {
  const index = args.indexOf(name)
  return index < 0 ? fallback : args[index + 1] ?? fallback
}
const files = args.flatMap((arg, index) => arg === '--file' ? [args[index + 1]] : [])
const apiOrigin = origin(value('--api'), 'API')
const repository = value('--repo')
const sourceSha = value('--source-sha', process.env.SCOPE_BUILD_COMMIT ?? '')
const workingTree = args.includes('--working-tree')
const token = process.env.SCOPE_MEDIA_SMOKE_TOKEN
const receiptPath = value('--receipt')
const timeoutSeconds = Number(value('--timeout-seconds', '300'))
assert(/^[^/]+\/[^/]+$/.test(repository), '--repo must be owner/repository')
assert(/^[0-9a-f]{40}$/.test(sourceSha), '--source-sha must identify the tested revision')
assert(!workingTree || ['localhost', '127.0.0.1', '[::1]'].includes(new URL(apiOrigin).hostname),
  '--working-tree is only supported against a local API')
assert(token, 'SCOPE_MEDIA_SMOKE_TOKEN is required')
assert(files.length > 0 && files.every(Boolean), 'at least one --file is required')
assert(Number.isFinite(timeoutSeconds) && timeoutSeconds > 0 && timeoutSeconds <= 1800,
  '--timeout-seconds must be between 1 and 1800')
const expectedMediaOrigin = value('--media-origin') ? origin(value('--media-origin'), 'media') : null
const repoPath = `/v1/repos/${repository.split('/').map(encodeURIComponent).join('/')}`
const receipt = {
  version: 2,
  source_sha: workingTree ? null : sourceSha,
  ...(workingTree ? { working_tree_parent_sha: sourceSha } : {}),
  api_origin: apiOrigin,
  repository,
  started_at: new Date().toISOString(),
  request_id: null,
  request_deleted: false,
  attachments: [],
  passed: false,
}
let failure

try {
  const request = await api(`${repoPath}/requests`, {
    name: `media-smoke-${randomUUID()}`,
    title: 'Request media integration fixture',
    audience: 'Private',
  })
  receipt.request_id = request.request.id
  const requestPath = `${repoPath}/requests/${encodeURIComponent(receipt.request_id)}`
  const attachmentsPath = `${requestPath}/attachments`
  let description = request.request.description_markdown
  for (const path of files) {
    const filename = basename(path)
    const mediaType = mediaTypeFor(filename)
    const size = (await stat(path)).size
    const digest = await hashFile(path)
    const intent = {
      operation_id: randomUUID(),
      target: { kind: 'Description', discussion_id: null },
      filename,
      declared_media_type: mediaType,
      size_bytes: size,
      sha256: digest,
    }
    const prepared = await api(`${attachmentsPath}/prepare`, intent)
    const mediaOrigin = origin(prepared.transfer.media_base_url, 'media')
    if (expectedMediaOrigin) assert.equal(mediaOrigin, expectedMediaOrigin)
    const receipts = []
    const partBytes = prepared.transfer.preferred_part_bytes
    assert(partBytes > 0 && partBytes <= 8 * 1024 * 1024, 'part size must remain bounded')
    const file = await open(path, 'r')
    try {
      for (let offset = 0; offset < size; offset += partBytes) {
        const bytes = Buffer.alloc(Math.min(partBytes, size - offset))
        const { bytesRead } = await file.read(bytes, 0, bytes.length, offset)
        assert.equal(bytesRead, bytes.length, 'fixture changed during transfer')
        const part = await putPart(prepared.transfer, receipts.length + 1, bytes)
        assert.equal(part.sha256, hash(bytes))
        assert.equal(part.size_bytes, bytes.length)
        receipts.push(part)
        if (receipts.length === 1) {
          const repeated = await putPart(prepared.transfer, 1, bytes)
          assert.deepEqual(repeated, part, 'duplicate part must be idempotent')
          const resumed = await api(`${attachmentsPath}/prepare`, intent)
          assert.equal(resumed.attachment.id, prepared.attachment.id)
          assert.equal(resumed.transfer.upload_id, prepared.transfer.upload_id)
          assert(resumed.transfer.acknowledged_parts.some((entry) => entry.part_number === 1 && entry.sha256 === part.sha256))
          prepared.transfer = resumed.transfer
        }
      }
    } finally {
      await file.close()
    }
    const attachmentPath = `${attachmentsPath}/${encodeURIComponent(prepared.attachment.id)}`
    const finishBody = { upload_id: prepared.transfer.upload_id, parts: receipts }
    await api(`${attachmentPath}/finish`, finishBody)
    const reference = `${mediaType.startsWith('image/') ? '!' : ''}[Fixture](/request-attachments/${prepared.attachment.id})`
    const nextDescription = `${description}\n\n${reference}`.trim()
    await api(requestPath, { description_markdown: nextDescription, expected_description_markdown: description }, 'PATCH')
    description = nextDescription
    const started = Date.now()
    let attachment
    do {
      attachment = await api(attachmentPath)
      if (attachment.state === 'Ready') break
      assert(!['Failed', 'Rejected'].includes(attachment.state),
        `${filename}: processing ${attachment.state}: ${attachment.failure?.message ?? ''}`)
      assert(Date.now() - started < timeoutSeconds * 1000, `${filename}: processing timed out`)
      await delay(1000)
    } while (true)
    const repeatedFinish = await api(`${attachmentPath}/finish`, finishBody)
    assert.equal(repeatedFinish.id, attachment.id)
    assert.equal(repeatedFinish.state, 'Ready', 'finish retry must preserve conversion')
    const original = await api(`${attachmentPath}/media-grant`, { target: { kind: 'original' } })
    assert.equal(new URL(original.media_url).origin, mediaOrigin)
    const head = await mediaFetch(original.media_url, { method: 'HEAD' })
    assert.equal(head.status, 200)
    assert.equal(Number(head.headers.get('content-length')), size)
    assert.equal(head.headers.get('accept-ranges'), 'bytes')
    assert.match(head.headers.get('cache-control') ?? '', /no-store|private/)
    assert(head.headers.get('etag'), 'original ETag is required')
    const playbackStartBegan = performance.now()
    const playbackStart = await mediaFetch(original.media_url, { headers: { Range: 'bytes=0-15' } })
    assert.equal(playbackStart.status, 206)
    assert.equal((await playbackStart.arrayBuffer()).byteLength, Math.min(16, size))
    const playbackStartMillis = performance.now() - playbackStartBegan
    const rangeStart = Math.min(size - 1, Math.max(0, partBytes - 3))
    const rangeEnd = Math.min(size - 1, rangeStart + 8)
    const seekBegan = performance.now()
    const range = await mediaFetch(original.media_url, { headers: { Range: `bytes=${rangeStart}-${rangeEnd}` } })
    assert.equal(range.status, 206)
    assert.equal(range.headers.get('content-range'), `bytes ${rangeStart}-${rangeEnd}/${size}`)
    const expected = Buffer.alloc(rangeEnd - rangeStart + 1)
    const local = await open(path, 'r')
    try { await local.read(expected, 0, expected.length, rangeStart) } finally { await local.close() }
    assert.deepEqual(Buffer.from(await range.arrayBuffer()), expected)
    const seekMillis = performance.now() - seekBegan
    const unsatisfiable = await mediaFetch(original.media_url, { headers: { Range: `bytes=${size}-` } })
    assert.equal(unsatisfiable.status, 416)
    assert.equal(unsatisfiable.headers.get('content-range'), `bytes */${size}`)
    const fullDownloadBegan = performance.now()
    const full = await mediaFetch(original.media_url)
    assert.equal(full.status, 200)
    const returnedDigest = createHash('sha256')
    for await (const bytes of full.body) returnedDigest.update(bytes)
    assert.equal(returnedDigest.digest('hex'), digest, 'original must round-trip exactly')
    const fullDownloadMillis = performance.now() - fullDownloadBegan
    const derivatives = []
    for (const derivative of attachment.derivatives) {
      const grant = await api(`${attachmentPath}/media-grant`, {
        target: { kind: 'derivative', derivative_id: derivative.id },
      })
      const response = await mediaFetch(grant.media_url, { headers: { Range: 'bytes=0-15' } })
      assert.equal(response.status, 206)
      assert.equal(response.headers.get('content-type'), derivative.media_type)
      assert.equal((await response.arrayBuffer()).byteLength, Math.min(16, derivative.size_bytes))
      derivatives.push({ id: derivative.id, kind: derivative.kind, size_bytes: derivative.size_bytes })
    }
    assert(derivatives.length > 0, 'processing must create a preview')
    const anonymous = await fetch(`${apiOrigin}${attachmentPath}`, { signal: AbortSignal.timeout(15000) })
    assert([401, 403, 404].includes(anonymous.status), 'private media metadata leaked')
    await anonymous.body?.cancel()
    receipt.attachments.push({
      id: attachment.id, filename, kind: attachment.kind, detected_media_type: attachment.detected_media_type,
      size_bytes: size, sha256: digest, media_origin: mediaOrigin, processing_millis: Date.now() - started,
      playback_start_millis: playbackStartMillis, seek_millis: seekMillis,
      full_download_millis: fullDownloadMillis,
      range_crossed_chunk_boundary: rangeStart < partBytes && rangeEnd >= partBytes,
      derivatives,
    })
  }
  if (args.includes('--require-video')) {
    assert(receipt.attachments.some(({ kind }) => kind === 'Photo'), 'photo fixture is required')
    assert(receipt.attachments.some(({ kind }) => kind === 'Video'), 'video fixture is required')
  }
  receipt.passed = true
} catch (error) {
  failure = error
} finally {
  if (receipt.request_id) {
    try {
      const result = await api(`${repoPath}/requests/${encodeURIComponent(receipt.request_id)}`, undefined, 'DELETE')
      assert.equal(result.deleted, true, 'smoke request must remain a deletable draft')
      receipt.request_deleted = true
    } catch (error) {
      failure ??= error
    }
  }
  receipt.passed &&= !failure
  receipt.finished_at = new Date().toISOString()
  if (receiptPath) {
    await mkdir(dirname(receiptPath), { recursive: true })
    await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`)
  }
  process.stdout.write(`${JSON.stringify(receipt, null, 2)}\n`)
  if (failure) {
    process.stderr.write(`media smoke failed: ${failure.message}\n`)
    process.exitCode = 1
  }
}

async function api(path, body, method = body === undefined ? 'GET' : 'POST') {
  const response = await fetch(`${apiOrigin}${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, 'x-scope-cli-protocol': '1', ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(30000),
  })
  const result = await response.json()
  assert(response.ok, `${method} ${path}: HTTP ${response.status}: ${result.message ?? 'request failed'}`)
  return result
}

async function putPart(transfer, number, bytes) {
  const response = await fetch(`${transfer.media_base_url}/v1/uploads/${encodeURIComponent(transfer.upload_id)}/parts/${number}`, {
    method: 'PUT', headers: { Authorization: `Bearer ${transfer.grant}`, 'Content-Type': 'application/octet-stream' },
    body: bytes, signal: AbortSignal.timeout(60000),
  })
  const result = await response.json()
  assert(response.ok, `part ${number}: HTTP ${response.status}: ${result.message ?? 'transfer failed'}`)
  return result
}

function mediaFetch(url, options = {}) {
  return fetch(url, { ...options, signal: AbortSignal.timeout(60000) }).catch(() => {
    throw new Error('media transport failed')
  })
}

function origin(value, label) {
  const url = new URL(value)
  assert(url.protocol === 'https:' || (url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)),
    `${label} must use HTTPS outside loopback`)
  assert(!url.username && !url.password && !url.search && !url.hash && url.pathname === '/', `${label} must be a plain origin`)
  return url.origin
}

function mediaTypeFor(filename) {
  const extension = filename.split('.').at(-1).toLowerCase()
  const mediaType = {
    png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', gif: 'image/gif',
    heic: 'image/heic', heif: 'image/heif', mp4: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm',
  }[extension]
  assert(mediaType, `unsupported fixture extension: ${extension}`)
  return mediaType
}

async function hashFile(path) {
  const digest = createHash('sha256')
  for await (const bytes of createReadStream(path)) digest.update(bytes)
  return digest.digest('hex')
}

function hash(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}
