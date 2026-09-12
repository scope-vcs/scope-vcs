import { mkdirSync, rmSync, writeFileSync } from 'node:fs'

// tsconfig.test.json emits CommonJS while package.json is type: module, so the
// emitted tests need their own package type.
rmSync('.test-output', { recursive: true, force: true })
mkdirSync('.test-output', { recursive: true })
writeFileSync('.test-output/package.json', '{"type":"commonjs"}\n')
