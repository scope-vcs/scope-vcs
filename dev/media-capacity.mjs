#!/usr/bin/env node

import { sampleRequest, summarize } from './media-capacity-samples.mjs'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, stat, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

const args = process.argv.slice(2)
const value = (name, fallback = '') => {
  const index = args.indexOf(name)
  return index < 0 ? fallback : args[index + 1] ?? fallback
}
const api = new URL(value('--api'))
const repository = value('--repo')
const sourceSha = value('--source-sha')
const workingTree = args.includes('--working-tree')
const largeVideo = value('--large-video')
const photo = value('--photo')
const output = resolve(value('--output', '.tmp/media-capacity/receipt.json'))
const gatewayPid = value('--gateway-pid')
const smallUploads = Number(value('--small-uploads', '4'))
assert(process.env.SCOPE_MEDIA_SMOKE_TOKEN, 'SCOPE_MEDIA_SMOKE_TOKEN is required')
assert(/^[0-9a-f]{40}$/.test(sourceSha), '--source-sha must identify the tested revision')
assert(!workingTree || ['localhost', '127.0.0.1', '[::1]'].includes(api.hostname),
  '--working-tree is only supported against a local API')
assert(/^[^/]+\/[^/]+$/.test(repository), '--repo must be owner/repository')
assert(api.protocol === 'https:' || (api.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(api.hostname)),
  '--api must use HTTPS outside loopback')
assert(!api.username && !api.password && api.pathname === '/' && !api.search && !api.hash,
  '--api must be a plain origin')
assert(Number.isInteger(smallUploads) && smallUploads >= 2 && smallUploads <= 10,
  '--small-uploads must be between 2 and 10')
assert(!gatewayPid || /^[1-9][0-9]*$/.test(gatewayPid), '--gateway-pid must be a local process ID')
const videoSize = (await stat(largeVideo)).size
assert(videoSize >= 490_000_000 && videoSize <= 500 * 1024 * 1024,
  '--large-video must contain 490 MB to 500 MiB of valid video')
assert((await stat(photo)).size > 0, '--photo must contain a valid photo')
await mkdir(dirname(output), { recursive: true })

const requestUrl = new URL(`/v1/repos/${repository.split('/').map(encodeURIComponent).join('/')}/requests`, api)
const baseline = []
for (let index = 0; index < 20; index += 1) baseline.push(await sample())
const loaded = []
const smokeScript = fileURLToPath(new URL('./media-smoke.mjs', import.meta.url))
let finished = false
const sampler = (async () => {
  while (!finished) {
    loaded.push(await sample())
    if (!finished) await delay(500)
  }
})()
const receipt = {
  version: 1,
  source_sha: workingTree ? null : sourceSha,
  ...(workingTree ? { working_tree_parent_sha: sourceSha } : {}),
  api_origin: api.origin,
  repository,
  started_at: new Date().toISOString(),
  large_video_bytes: videoSize,
  small_uploads: smallUploads,
  concurrent_smoke_flows: 3,
  passed: false,
}

try {
  const jobs = [largeVideo, ...Array.from({ length: smallUploads }, () => photo)]
  let cursor = 0
  const outcomes = await Promise.allSettled(Array.from({ length: 3 }, async () => {
    while (cursor < jobs.length) {
      const index = cursor++
      await runSmoke(jobs[index], index)
    }
  }))
  const errors = outcomes.filter(({ status }) => status === 'rejected')
  receipt.flow_failures = errors.map(({ reason }) => reason.message)
  receipt.passed = errors.length === 0
} finally {
  finished = true
  await sampler
  receipt.finished_at = new Date().toISOString()
  receipt.baseline = summarize(baseline)
  receipt.loaded = summarize(loaded)
  receipt.samples = { baseline, loaded }
  receipt.passed &&= receipt.baseline.failed_requests === 0 && receipt.loaded.failed_requests === 0
  await writeFile(output, `${JSON.stringify(receipt, null, 2)}\n`)
}
process.stdout.write(`${JSON.stringify({ ...receipt, samples: undefined, receipt_path: output }, null, 2)}\n`)
if (!receipt.passed) process.exitCode = 1

function sample() {
  return sampleRequest(requestUrl, process.env.SCOPE_MEDIA_SMOKE_TOKEN, gatewayPid)
}

async function runSmoke(file, index) {
  const childArgs = [smokeScript, '--api', api.origin, '--repo', repository, '--source-sha', sourceSha,
    '--file', file, '--timeout-seconds', '1800', '--receipt', resolve(dirname(output), `flow-${index}.json`)]
  if (value('--media-origin')) childArgs.push('--media-origin', value('--media-origin'))
  if (workingTree) childArgs.push('--working-tree')
  const child = spawn(process.execPath, childArgs, { stdio: ['ignore', 'ignore', 'pipe'] })
  let diagnostic = ''
  child.stderr.setEncoding('utf8').on('data', (chunk) => { diagnostic = (diagnostic + chunk).slice(-4096) })
  return new Promise((resolve, reject) => {
    child.on('error', reject)
    child.on('exit', (code, signal) => {
      if (code === 0) resolve()
      else reject(new Error(`flow ${index} failed (${signal ?? code}): ${diagnostic.trim()}`))
    })
  })
}
