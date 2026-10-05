import { readFile, readdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { resourceBoundaryViolations } from './resource-boundary.mjs'
import { callbackAssertionViolations } from './callback-assertions.mjs'

const root = fileURLToPath(new URL('../', import.meta.url))
const violations = []
for (const file of await readdir(path.join(root, 'src'), { recursive: true })) {
  if (!/\.[jt]sx?$/.test(file) || /\.(generated|gen)\.[jt]sx?$/.test(file)) continue
  const relative = `src/${file}`
  const source = await readFile(path.join(root, relative), 'utf8')
  const check = /\.test\.[jt]sx?$/.test(file) ? callbackAssertionViolations : resourceBoundaryViolations
  violations.push(...check(relative, source))
}
if (violations.length) {
  console.error(violations.join('\n'))
  process.exitCode = 1
}
