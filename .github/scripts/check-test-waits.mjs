import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

const execFileAsync = promisify(execFile)

export const maxFixedWaitMs = 5_000

const scriptExtensions = new Set(['.cjs', '.js', '.jsx', '.mjs', '.ts', '.tsx'])
const testTokens = new Set(['test', 'tests', 'spec'])
const testSegments = new Set(['tests', 'test_support', 'smoke'])
const guardCallback = /\b(reject|throw|abort|kill)\b/
const rustDurationScale = {
  from_nanos: 1e-6,
  from_micros: 1e-3,
  from_millis: 1,
  from_secs: 1_000,
  from_secs_f32: 1_000,
  from_secs_f64: 1_000,
  from_mins: 60_000,
  from_hours: 3_600_000,
}
const shellSuffixScale = { '': 1_000, s: 1_000, m: 60_000, h: 3_600_000, d: 86_400_000 }

function isTestPath(normalized) {
  const segments = normalized.split('/')
  const basename = segments.pop()
  const tokens = basename.split(/[-_.]/)
  return tokens.some((token) => testTokens.has(token))
    || segments.some((segment) => testSegments.has(segment) || segment.endsWith('_tests'))
}

export function testLanguage(file) {
  const normalized = file.replaceAll('\\', '/')
  const extension = path.posix.extname(normalized)
  const testPath = isTestPath(normalized)
  if (extension === '.rs') return testPath ? 'rust' : 'rust-inline'
  if (!testPath) return null
  if (scriptExtensions.has(extension)) return 'script'
  if (extension === '.py') return 'python'
  if (extension === '.sh' || extension === '.bash') return 'shell'
  return null
}

function literalNumber(text) {
  const trimmed = text.trim()
  return /^\d[\d_]*(\.[\d_]+)?$/.test(trimmed) ? Number(trimmed.replaceAll('_', '')) : null
}

function callArguments(contents, start) {
  const args = []
  let depth = 1
  let current = ''
  let index = start
  for (; index < contents.length && depth > 0; index += 1) {
    const character = contents[index]
    if (character === '(' || character === '[' || character === '{') depth += 1
    if (character === ')' || character === ']' || character === '}') depth -= 1
    if (depth === 0) break
    if (character === ',' && depth === 1) {
      args.push(current)
      current = ''
    } else {
      current += character
    }
  }
  args.push(current)
  return args.map((arg) => arg.replace(/\s+/g, ' ').trim())
}

function* calls(contents, pattern) {
  for (const match of contents.matchAll(pattern)) {
    const line = contents.slice(0, match.index).split('\n').length
    yield { name: match[1], line, args: callArguments(contents, match.index + match[0].length) }
  }
}

function realClockLines(lines, inlineOnly) {
  let inTest = !inlineOnly
  let paused = false
  return lines.map((line, index) => {
    const trimmed = line.trim()
    if (inlineOnly && trimmed === '#[cfg(test)]' && /^\s*mod \w+ \{/.test(lines[index + 1] ?? '')) inTest = true
    if (/^#\[(tokio::)?test\b/.test(trimmed)) paused = /start_paused\s*=\s*true/.test(trimmed)
    if (/\btime::pause\(\)/.test(trimmed)) paused = true
    if (/\btime::resume\(\)/.test(trimmed)) paused = false
    return inTest && !paused
  })
}

function rustWaitMs(argument) {
  const constructor = argument.match(/^(?:std::time::|tokio::time::)?Duration::(\w+)\((.*)\)$/)
  if (!constructor) return 0
  const [, name, inner] = constructor
  const value = literalNumber(name === 'new' ? inner.split(',')[0] : inner)
  if (name === 'new') return value === null ? 0 : value * 1_000
  const scale = rustDurationScale[name]
  if (!scale) return 0
  if (value !== null) return value * scale
  return scale >= 1_000 ? Infinity : 0
}

function rustWaits(contents, inlineOnly) {
  const realClock = realClockLines(contents.split('\n'), inlineOnly)
  return [...calls(contents, /\b(sleep)\(/g)]
    .filter(({ line }) => realClock[line - 1])
    .map(({ line, args }) => ({ line, waitMs: rustWaitMs(args.join(', ')), text: `sleep(${args.join(', ')})` }))
}

function scriptWaitMs({ name, args }) {
  if (name === 'setTimeout' && args.length >= 2) {
    return guardCallback.test(args[0]) ? 0 : literalNumber(args[1]) ?? 0
  }
  return literalNumber(args[0]) ?? 0
}

function scriptWaits(contents) {
  return [...calls(contents, /\b(waitForTimeout|setTimeout|delay|sleep)\(/g)]
    .map((call) => ({ line: call.line, waitMs: scriptWaitMs(call), text: `${call.name}(${call.args.join(', ')})` }))
}

function pythonWaits(contents) {
  return [...calls(contents, /\b(sleep)\(/g)]
    .map(({ line, args }) => ({ line, waitMs: (literalNumber(args[0]) ?? 0) * 1_000, text: `sleep(${args.join(', ')})` }))
}

function shellWaits(contents) {
  return contents.split('\n').flatMap((text, index) =>
    [...text.matchAll(/(?:^|[\s;&|(])(?:\/bin\/)?sleep\s+(\d+(?:\.\d+)?)([smhd]?)(?=$|[\s;&|)])/g)]
      .map((match) => ({ line: index + 1, waitMs: Number(match[1]) * shellSuffixScale[match[2]], text: text.trim() })))
}

export function findLongWaits(file, contents) {
  const language = testLanguage(file)
  const waits = language === 'rust' ? rustWaits(contents, false)
    : language === 'rust-inline' ? rustWaits(contents, true)
      : language === 'script' ? scriptWaits(contents)
        : language === 'python' ? pythonWaits(contents)
          : language === 'shell' ? shellWaits(contents)
            : []
  return waits
    .filter(({ waitMs }) => waitMs >= maxFixedWaitMs)
    .map(({ line, waitMs, text }) => ({ file, line, waitMs, text }))
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
