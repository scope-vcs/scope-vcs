import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { PNG } from 'pngjs'
import pixelmatch from 'pixelmatch'

export function visualBaseline(suite) {
  const snapshots = new URL(`../snapshots/${suite}/`, import.meta.url)
  const artifacts = new URL(`../../.tmp/${suite}/`, import.meta.url)
  return async function compareScreenshot(locator, name) {
    const actualBytes = await locator.screenshot({ animations: 'disabled' })
    if (process.env.SCOPE_UPDATE_VISUAL_SNAPSHOTS === '1') {
      assert(!process.env.CI, 'CI cannot update visual baselines')
      await mkdir(snapshots, { recursive: true })
      await writeFile(new URL(`${name}.png`, snapshots), actualBytes)
      return
    }
    const actual = PNG.sync.read(actualBytes)
    const expected = PNG.sync.read(await readFile(new URL(`${name}.png`, snapshots)))
    await mkdir(artifacts, { recursive: true })
    await writeFile(new URL(`${name}-actual.png`, artifacts), actualBytes)
    assert.deepEqual([actual.width, actual.height], [expected.width, expected.height], `${name}: layout dimensions changed`)
    const diff = new PNG({ width: actual.width, height: actual.height })
    const changed = pixelmatch(expected.data, actual.data, diff.data, actual.width, actual.height, { threshold: 0.15 })
    if (changed) await writeFile(new URL(`${name}-diff.png`, artifacts), PNG.sync.write(diff))
    assert.equal(changed, 0, `${name}: rendered pixels changed; inspect web/.tmp/${suite} before updating the baseline`)
  }
}
