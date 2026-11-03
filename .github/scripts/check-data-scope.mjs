import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

const execFileAsync = promisify(execFile)

const database = 'crates/scope-postgres/src/db'
const migrations = 'crates/scope-postgres/src/migrations'

export const rules = [
  {
    name: 'raw database connection',
    pattern: /\b(?:DatabaseConnection|TransactionTrait|begin_with_config|get_postgres_connection_pool|SqlxPostgresConnector)\b/,
    owners: [
      `${database}/data_scope.rs`,
      `${database}/connection.rs`,
      `${database}/test_support.rs`,
      `${database}/test_support/`,
      `${database}/migration_tests.rs`,
      `${database}/migration_tests/`,
      `${migrations}/`,
    ],
    reason: 'open transactions through ScopedDatabase::begin with a DataScope',
  },
  {
    name: 'data scope setting',
    pattern: /\bscope\.(?:repository_id|system_reason)\b/,
    owners: [
      `${database}/data_scope.rs`,
      `${database}/repository_row_security_tests.rs`,
      `${migrations}/m0107_repository_row_security.rs`,
    ],
    reason: 'only the DataScope boundary sets or reads the row security settings',
  },
]

const owns = (owners, file) => owners.some((owner) => (owner.endsWith('/') ? file.startsWith(owner) : file === owner))

export function findViolations(file, contents) {
  const normalized = file.replaceAll('\\', '/')
  if (!normalized.endsWith('.rs')) return []
  const violations = []
  for (const rule of rules) {
    if (owns(rule.owners, normalized)) continue
    contents.split('\n').forEach((text, index) => {
      if (rule.pattern.test(text)) violations.push({ file: normalized, line: index + 1, rule, text: text.trim() })
    })
  }
  return violations
}

async function repositoryFiles(root) {
  const { stdout } = await execFileAsync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
    { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 },
  )
  return stdout.split('\0').filter(Boolean)
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const violations = []
  for (const file of await repositoryFiles(root)) {
    if (!file.endsWith('.rs')) continue
    let contents
    try {
      contents = await readFile(path.join(root, file), 'utf8')
    } catch (error) {
      if (error.code === 'ENOENT' || error.code === 'EISDIR') continue
      throw error
    }
    violations.push(...findViolations(file, contents))
  }
  if (violations.length > 0) {
    console.error('Data scope guardrail failed. Every PostgreSQL transaction must declare its DataScope at crates/scope-postgres/src/db/data_scope.rs.')
    for (const { file, line, rule, text } of violations) console.error(`  ${file}:${line}: ${rule.name}: ${text} (${rule.reason})`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
