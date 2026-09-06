import { readFile, readdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { resourceBoundaryViolations } from './resource-boundary.mjs'

const root = fileURLToPath(new URL('../', import.meta.url))
const violations = []
for (const directory of ['src/components', 'src/features']) {
  for (const file of await readdir(path.join(root, directory), { recursive: true })) {
    if (!/\.[jt]sx?$/.test(file) || /\.test\.[jt]sx?$/.test(file)) continue
    const relative = `${directory}/${file}`
    violations.push(...resourceBoundaryViolations(relative, await readFile(path.join(root, relative), 'utf8')))
  }
}
if (violations.length) {
  console.error(violations.join('\n'))
  process.exitCode = 1
}
