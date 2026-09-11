import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import test from 'node:test';
import { chromium } from 'playwright';
import { observeEventStreams } from './event-stream-evidence.mjs';

test('browser stream evidence requires successful SSE headers and a complete delivered frame', async () => {
  const responses = new Set();
  const server = createServer((request, response) => {
    if (request.url === '/') return response.end('<title>SSE evidence fixture</title>');
    responses.add(response);
    const mode = new URL(request.url, 'http://localhost').searchParams.get('mode');
    if (mode === 'hanging') return;
    response.writeHead(mode === 'rejected' ? 503 : 200, { 'content-type': mode === 'json' ? 'application/json' : 'text/event-stream' });
    response.flushHeaders();
    if (mode !== 'silent') response.write(mode === 'partial' ? 'data: pending\n' : ': heartbeat\n\n');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    const starts = [], ends = [];
    await observeEventStreams(page, '/events', starts, ends);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    for (const mode of ['hanging', 'rejected', 'json', 'silent', 'partial', 'valid']) {
      const expectedCount = starts.length + 1;
      await page.evaluate(mode => { void fetch(`/events?mode=${mode}`).then(async response => {
        const reader = response.body.getReader();
        while (!(await reader.read()).done) { /* consume the live application stream */ }
      }).catch(() => {}); }, mode);
      const deadline = Date.now() + 3000;
      while (starts.length < expectedCount || (mode === 'valid' && !starts.at(-1).confirmedAt)) {
        if (Date.now() > deadline) throw new Error(`Missing browser evidence for ${mode}`);
        await new Promise(resolve => setTimeout(resolve, 20));
      }
      await page.waitForTimeout(100);
      assert.equal(Boolean(starts.at(-1).confirmedAt), mode === 'valid', mode);
    }
    assert.equal(starts.length, 6);
  } finally {
    await browser.close();
    for (const response of responses) response.destroy();
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
});
