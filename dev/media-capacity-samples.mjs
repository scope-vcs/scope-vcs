import { readFile } from 'node:fs/promises'

export async function sampleRequest(requestUrl, token, gatewayPid, timeoutMs = 15000) {
  const started = performance.now()
  let status = 0
  let completed = false
  let error = null
  try {
    const response = await fetch(requestUrl, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(timeoutMs),
    })
    status = response.status
    await response.arrayBuffer()
    completed = true
  } catch (cause) {
    error = String(cause.message || cause).slice(0, 500)
  }
  let gatewayRssBytes = null
  if (gatewayPid) {
    const status = await readFile(`/proc/${gatewayPid}/status`, 'utf8').catch(() => '')
    const kib = status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1]
    if (kib) gatewayRssBytes = Number(kib) * 1024
  }
  return { at: new Date().toISOString(), status, completed, error, latency_ms: performance.now() - started, gateway_rss_bytes: gatewayRssBytes }
}

export function summarize(samples) {
  const latencies = samples.map(({ latency_ms }) => latency_ms).sort((a, b) => a - b)
  const memory = samples.flatMap(({ gateway_rss_bytes }) => gateway_rss_bytes === null ? [] : [gateway_rss_bytes])
  return {
    requests: samples.length,
    failed_requests: samples.filter(({ status, completed }) => status !== 200 || !completed).length,
    p50_ms: latencies[Math.floor(latencies.length * 0.5)] ?? null,
    p95_ms: latencies[Math.min(latencies.length - 1, Math.floor(latencies.length * 0.95))] ?? null,
    max_ms: latencies.at(-1) ?? null,
    peak_gateway_rss_bytes: memory.length ? Math.max(...memory) : null,
  }
}
