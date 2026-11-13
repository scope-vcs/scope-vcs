import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { repositorySources } from './repository-files.mjs'

const slashExtensions = new Set(['.cjs', '.css', '.js', '.jsx', '.mjs', '.rs', '.ts', '.tsx'])
const hashExtensions = new Set(['.bash', '.example', '.py', '.sh', '.toml', '.yaml', '.yml'])
const exemptPrefixes = ['legal/']
const migrationDir = 'crates/scope-postgres/src/migrations/'
const htmlExtensions = new Set(['.htm', '.html'])
const legalNotice = /\b(copyright|licen[cs]e)\b/i
const generatedSuffixes = ['.gen.ts', '.generated.ts']

const slashDirectives = [
  /^\/\*\s*globals?\s/,
  /^\/\/\/\s*<reference /,
]
const hashDirectives = [
  /^#!/,
  /^#\s*shellcheck\s/,
  /^#\s*syntax=/,
  /^#\s*(noqa|type:\s*ignore|pragma)\b/,
]

export function commentSyntax(file, firstLine = '') {
  const normalized = file.replaceAll('\\', '/')
  const basename = path.posix.basename(normalized)
  if (exemptPrefixes.some((prefix) => normalized.startsWith(prefix))) return null
  if (normalized.startsWith(migrationDir) && /^m\d+_.*\.rs$/.test(basename)) return null
  if (generatedSuffixes.some((suffix) => basename.endsWith(suffix))) return null
  const extension = path.posix.extname(basename)
  if (slashExtensions.has(extension)) return 'slash'
  if (htmlExtensions.has(extension)) return 'html'
  if (hashExtensions.has(extension) || basename.startsWith('Dockerfile')) return 'hash'
  if (extension === '' && firstLine.startsWith('#!')) return 'hash'
  return null
}

function isSlashComment(trimmed) {
  return trimmed.startsWith('//') || /^\{?\/\*(?!\))/.test(trimmed)
}

function htmlComments(file, lines) {
  const found = []
  lines.forEach((line, index) => {
    const trimmed = line.trim()
    if (!trimmed.startsWith('<!--')) return
    const closing = lines.findIndex((candidate, candidateIndex) =>
      candidateIndex >= index && candidate.includes('-->'))
    const block = lines.slice(index, closing === -1 ? lines.length : closing + 1).join('\n')
    if (!legalNotice.test(block)) found.push({ file, line: index + 1, text: trimmed })
  })
  return found
}

export function findComments(file, contents) {
  const lines = contents.split('\n')
  const syntax = commentSyntax(file, lines[0])
  if (!syntax) return []
  if (syntax === 'html') return htmlComments(file, lines)
  const found = []
  lines.forEach((line, index) => {
    const trimmed = line.trim()
    if (syntax === 'hash') {
      const isComment = trimmed.startsWith('#')
      if (isComment && !hashDirectives.some((directive) => directive.test(trimmed))) {
        found.push({ file, line: index + 1, text: trimmed })
      }
      return
    }
    if (!isSlashComment(trimmed)) return
    if (slashDirectives.some((directive) => directive.test(trimmed))) return
    found.push({ file, line: index + 1, text: trimmed })
  })
  return found
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const comments = []
  for await (const { file, contents } of repositorySources(root)) {
    comments.push(...findComments(file, contents))
  }
  if (comments.length > 0) {
    console.error('Comment guardrail failed. AGENTS.md: express intent in code, not comments.')
    for (const { file, line, text } of comments) console.error(`  ${file}:${line}: ${text}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
