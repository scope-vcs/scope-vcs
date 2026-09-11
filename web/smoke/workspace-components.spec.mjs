import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import tailwindcss from '@tailwindcss/vite';

const require = createRequire(import.meta.url);
test('retained HTML preview survives theme changes and touch users can close inactive tabs', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-components-'));
  t.after(() => rm(cacheDir, { recursive: true, force: true }));
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/workspace', import.meta.url)),
    plugins: [tailwindcss()],
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react'].map(name => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  });
  await server.listen();
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ hasTouch: true, viewport: { width: 1024, height: 768 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(server.resolvedUrls.local[0]);
    await page.locator('iframe').waitFor();
    const first = await page.locator('iframe').elementHandle();
    await page.getByRole('button', { name: 'Toggle source' }).click();
    await page.locator('pre').waitFor();
    await page.getByRole('button', { name: 'Toggle source' }).click();
    await page.locator('pre').waitFor({ state: 'detached' });
    assert.equal(await first.evaluate(node => node === document.querySelector('iframe')), true);
    await page.getByRole('button', { name: 'Toggle theme' }).click();
    await page.waitForFunction(() => document.querySelector('iframe')?.contentDocument !== undefined);
    await page.frameLocator('iframe').getByRole('heading', { name: 'Test preview' }).waitFor();
    assert.equal(await first.evaluate(node => node.isConnected), false);
    assert.equal(await page.locator('iframe').count(), 1);
    const close = page.getByRole('button', { name: 'Close Two.html' });
    assert.equal(await close.evaluate(node => getComputedStyle(node).opacity), '1');
    await close.tap();
    assert.equal(await page.getByRole('tab', { name: 'Two.html' }).count(), 0);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
    await server.close();
  }
});
