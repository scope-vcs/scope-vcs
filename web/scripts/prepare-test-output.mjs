import { mkdirSync, rmSync, writeFileSync } from 'node:fs'

rmSync('.test-output', { recursive: true, force: true })
mkdirSync('.test-output', { recursive: true })
writeFileSync('.test-output/package.json', '{"type":"commonjs"}\n')
