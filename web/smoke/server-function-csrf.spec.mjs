import assert from 'node:assert/strict'
import { test } from 'node:test'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? 'http://localhost:3000'
const serverFunctionUrl = new URL('/_serverFn/invalid', baseUrl)
// Forwarded headers are client-controlled, so they must not move the origin the server expects.
const forgedHost = 'another.example'
const forgedHostOrigin = `${serverFunctionUrl.protocol}//${forgedHost}`
const forgedProtocol = serverFunctionUrl.protocol === 'https:' ? 'http' : 'https'

test('server function requests require same-origin browser metadata', async () => {
  for (const headers of [
    { 'Sec-Fetch-Site': 'cross-site' },
    { 'Sec-Fetch-Site': 'cross-site', Origin: serverFunctionUrl.origin },
    { Origin: 'https://another.example' },
    { Referer: 'https://another.example/page' },
    { 'X-Forwarded-Host': forgedHost, Origin: forgedHostOrigin },
    { Forwarded: `host=${forgedHost}`, Origin: forgedHostOrigin },
    { 'X-Forwarded-Proto': forgedProtocol, Origin: `${forgedProtocol}://${serverFunctionUrl.host}` },
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
