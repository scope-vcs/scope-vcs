import assert from 'node:assert/strict'
import { test } from 'node:test'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? 'http://localhost:3000'
const serverFunctionUrl = new URL('/_serverFn/invalid', baseUrl)

test('server function requests require same-origin browser metadata', async () => {
  for (const headers of [
    { 'Sec-Fetch-Site': 'cross-site' },
    { Origin: 'https://another.example' },
    {},
  ]) {
    const response = await fetch(serverFunctionUrl, { headers })
    assert.equal(response.status, 403, JSON.stringify(headers))
  }

  for (const headers of [
    { 'Sec-Fetch-Site': 'same-origin' },
    { Origin: serverFunctionUrl.origin },
  ]) {
    const response = await fetch(serverFunctionUrl, { headers })
    assert.notEqual(response.status, 403, JSON.stringify(headers))
  }
})
