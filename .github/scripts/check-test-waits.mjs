import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

const execFileAsync = promisify(execFile)

export const maxFixedWaitMs = 5_000

const number = String.raw`([\d_]+(?:\.[\d_]+)?)`
const toNumber = (text) => Number(text.replaceAll('_', ''))

const rustDuration = new RegExp(String.raw`^(?:std::time::|tokio::time::)?Duration::from_(secs|millis|secs_f32|secs_f64)\(\s*${number}\s*\)$`)
const scriptWaits = [
  new RegExp(String.raw`waitForTimeout\(\s*${number}\s*\)`, 'g'),
  new RegExp(String.raw`setTimeout\(\s*(?:resolve|res|r|done)\s*,\s*${number}\s*\)`, 'g'),
  new RegExp(String.raw`\b(?:delay|sleep|setTimeout)\(\s*${number}\s*[,)]`, 'g'),
]
const pythonWait = new RegExp(String.raw`\bsleep\(\s*${number}\s*\)`, 'g')
const shellWait = new RegExp(String.raw`(?:^|[\s;&|(])(?:/bin/)?sleep\s+${number}\b`, 'g')

const scriptExtensions = new Set(['.cjs', '.js', '.jsx', '.mjs', '.ts', '.tsx'])

export function testLanguage(file) {
  const normalized = file.replaceAll('\\', '/')
  const basename = path.posix.basename(normalized)
  const extension = path.posix.extname(basename)
  const segments = normalized.split('/')
  if (extension === '.rs') {
    const testPath = segments.some((segment) =>
      segment === 'tests' || segment === 'test_support' || segment.endsWith('_tests'))
    return testPath || basename === 'tests.rs' || basename.endsWith('_tests.rs') ? 'rust' : 'rust-inline'
  }
  if (scriptExtensions.has(extension)) {
    const testPath = /\.(test|spec)\.[cm]?[jt]sx?$/.test(basename) || normalized.startsWith('web/smoke/')
    return testPath ? 'script' : null
  }
  if (extension === '.py') return /^test_.*\.py$/.test(basename) ? 'python' : null
  if (extension === '.sh' || extension === '.bash') {
    return /(^|[-_])test([-_.]|$)/.test(basename.replace(/\.(sh|bash)$/, '')) ? 'shell' : null
  }
  return null
}

function realClockLines(lines, inlineOnly) {
  let inTest = !inlineOnly
  let paused = false
  return lines.map((line, index) => {
    const trimmed = line.trim()
    if (inlineOnly && trimmed === '#[cfg(test)]' && /^\s*mod \w+ \{/.test(lines[index + 1] ?? '')) inTest = true
    if (/^#\[(tokio::)?test\b/.test(trimmed)) paused = /start_paused\s*=\s*true/.test(trimmed)
    if (/\btime::pause\(\)/.test(trimmed)) paused = true
    return inTest && !paused
  })
}

function sleepArgument(contents, start) {
  let depth = 1
  let end = start
  while (end < contents.length && depth > 0) {
    if (contents[end] === '(') depth += 1
    else if (contents[end] === ')') depth -= 1
    end += 1
  }
  return contents.slice(start, end - 1).replace(/\s+/g, ' ').trim()
}

function rustWaitMs(argument) {
  const literal = argument.match(rustDuration)
  if (literal) return (literal[1] === 'millis' ? 1 : 1_000) * toNumber(literal[2])
  return /^(?:std::time::|tokio::time::)?Duration::from_secs\(/.test(argument) ? Infinity : 0
}

function rustWaits(file, contents, inlineOnly) {
  const lines = contents.split('\n')
  const realClock = realClockLines(lines, inlineOnly)
  const found = []
  for (const match of contents.matchAll(/\bsleep\(/g)) {
    const index = contents.slice(0, match.index).split('\n').length - 1
    if (!realClock[index]) continue
    const argument = sleepArgument(contents, match.index + match[0].length)
    const waitMs = rustWaitMs(argument)
    if (waitMs >= maxFixedWaitMs) found.push({ file, line: index + 1, waitMs, text: `sleep(${argument})` })
  }
  return found
}

function patternWaits(file, lines, patterns, scale) {
  const found = []
  lines.forEach((line, index) => {
    for (const pattern of patterns) {
      for (const match of line.matchAll(pattern)) {
        const waitMs = toNumber(match[1]) * scale
        if (waitMs >= maxFixedWaitMs) found.push({ file, line: index + 1, waitMs, text: line.trim() })
      }
    }
  })
  return found
}

export function findLongWaits(file, contents) {
  const language = testLanguage(file)
  if (!language) return []
  if (language === 'rust') return rustWaits(file, contents, false)
  if (language === 'rust-inline') return rustWaits(file, contents, true)
  const lines = contents.split('\n')
  if (language === 'script') return patternWaits(file, lines, scriptWaits, 1)
  if (language === 'python') return patternWaits(file, lines, [pythonWait], 1_000)
  return patternWaits(file, lines, [shellWait], 1_000)
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
  const waits = []
  for (const file of await repositoryFiles(root)) {
    if (!testLanguage(file)) continue
    let contents
    try {
      contents = await readFile(path.join(root, file), 'utf8')
    } catch (error) {
      if (error.code === 'ENOENT' || error.code === 'EISDIR') continue
      throw error
    }
    waits.push(...findLongWaits(file, contents))
  }
  if (waits.length > 0) {
    console.error(`Test wait guardrail failed. Tests may not wait ${maxFixedWaitMs} ms or more, or a computed number of seconds, on the real clock. Use a paused or injected clock, or wait for the event. See .agents/skills/scope-tests/SKILL.md.`)
    for (const { file, line, text } of waits) console.error(`  ${file}:${line}: ${text}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
