import assert from 'node:assert/strict'
import { test } from 'node:test'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? 'http://localhost:3000'
const serverFunctionUrl = new URL('/_serverFn/invalid', baseUrl)

test('server function requests require same-origin browser metadata', async () => {
  for (const headers of [
    { 'Sec-Fetch-Site': 'cross-site' },
    { 'Sec-Fetch-Site': 'cross-site', Origin: serverFunctionUrl.origin },
    { Origin: 'https://another.example' },
    { Referer: 'https://another.example/page' },
    {},
  ]) {
    const response = await fetch(serverFunctionUrl, { headers })
    assert.equal(response.status, 403, JSON.stringify(headers))
  }

  for (const headers of [
    { 'Sec-Fetch-Site': 'same-origin' },
    { Origin: serverFunctionUrl.origin },
    { Referer: new URL('/repositories', baseUrl).href },
  ]) {
    const response = await fetch(serverFunctionUrl, { headers })
    assert.notEqual(response.status, 403, JSON.stringify(headers))
  }

  if (serverFunctionUrl.protocol === 'https:') {
    const response = await fetch(serverFunctionUrl, {
      headers: { Origin: `http://${serverFunctionUrl.host}` },
    })
    assert.equal(response.status, 403, 'rejects an insecure origin at an HTTPS deployment')
  }
})
