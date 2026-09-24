import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { conventionViolations } from './conventions.mjs'

const root = fileURLToPath(new URL('../', import.meta.url))
const violations = []
for (const directory of ['src/routes', 'src/features']) {
  for (const file of await readdir(path.join(root, directory), { recursive: true })) {
    if (!file.endsWith('.tsx')) continue
    const relative = `${directory}/${file}`
    violations.push(...conventionViolations(relative, await readFile(path.join(root, relative), 'utf8')))
  }
}
if (violations.length) {
  console.error(violations.join('\n'))
  process.exitCode = 1
}
