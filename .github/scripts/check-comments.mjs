import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

const execFileAsync = promisify(execFile)

const slashExtensions = new Set(['.cjs', '.css', '.js', '.jsx', '.mjs', '.rs', '.ts', '.tsx'])
const hashExtensions = new Set(['.bash', '.py', '.sh', '.toml', '.yaml', '.yml'])
const exemptPrefixes = ['crates/scope-postgres/src/migrations/', 'legal/']
const generatedSuffixes = ['.gen.ts', '.generated.ts']

const slashDirectives = [
  /^\/\/\s*SAFETY:/,
  /^\/\/\s*@ts-/,
  /^\/\/\s*(eslint|oxlint)-/,
  /^\/\*\s*(eslint|oxlint)[-\s]/,
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
  if (generatedSuffixes.some((suffix) => basename.endsWith(suffix))) return null
  const extension = path.posix.extname(basename)
  if (slashExtensions.has(extension)) return 'slash'
  if (hashExtensions.has(extension) || basename.startsWith('Dockerfile')) return 'hash'
  if (extension === '' && firstLine.startsWith('#!')) return 'hash'
  return null
}

function isSlashComment(trimmed) {
  return trimmed.startsWith('//') || /^\{?\/\*(?!\))/.test(trimmed)
}

const isDocLine = (trimmed) => /^\/\/\/(\s|$)/.test(trimmed)

function braceDelta(line) {
  return [...line].reduce((depth, character) =>
    depth + (character === '{' ? 1 : character === '}' ? -1 : 0), 0)
}

export function rustContractDocLines(lines) {
  const allowed = new Set()
  let pendingDocs = []
  let attributes = ''
  let contractDepth = 0
  lines.forEach((line, index) => {
    const trimmed = line.trim()
    if (contractDepth > 0) {
      if (isDocLine(trimmed)) allowed.add(index)
      else contractDepth += braceDelta(trimmed)
      return
    }
    if (isDocLine(trimmed)) {
      pendingDocs.push(index)
      return
    }
    if (trimmed.startsWith('#[') || (attributes && !attributes.trimEnd().endsWith(']'))) {
      attributes += trimmed
      return
    }
    if (/\bJsonSchema\b/.test(attributes) && trimmed !== '') {
      pendingDocs.forEach((docIndex) => allowed.add(docIndex))
      contractDepth = Math.max(braceDelta(trimmed), 0)
    }
    pendingDocs = []
    attributes = ''
  })
  return allowed
}

export function findComments(file, contents) {
  const lines = contents.split('\n')
  const syntax = commentSyntax(file, lines[0])
  if (!syntax) return []
  const contractDocLines = file.endsWith('.rs') ? rustContractDocLines(lines) : new Set()
  const found = []
  let continuingSafety = false
  lines.forEach((line, index) => {
    const trimmed = line.trim()
    if (syntax === 'hash') {
      const isComment = trimmed.startsWith('#')
      if (isComment && !hashDirectives.some((directive) => directive.test(trimmed))) {
        found.push({ file, line: index + 1, text: trimmed })
      }
      return
    }
    if (!isSlashComment(trimmed)) {
      continuingSafety = false
      return
    }
    if (/^\/\/\s*SAFETY:/.test(trimmed)) {
      continuingSafety = true
      return
    }
    if (continuingSafety && trimmed.startsWith('//') && !trimmed.startsWith('///')) return
    continuingSafety = false
    if (contractDocLines.has(index)) return
    if (slashDirectives.some((directive) => directive.test(trimmed))) return
    found.push({ file, line: index + 1, text: trimmed })
  })
  return found
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
  const comments = []
  for (const file of await repositoryFiles(root)) {
    let contents
    try {
      contents = await readFile(path.join(root, file), 'utf8')
    } catch (error) {
      if (error.code === 'ENOENT' || error.code === 'EISDIR') continue
      throw error
    }
    comments.push(...findComments(file, contents))
  }
  if (comments.length > 0) {
    console.error('Comment guardrail failed. AGENTS.md: express intent in code instead of comments.')
    for (const { file, line, text } of comments) console.error(`  ${file}:${line}: ${text}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
