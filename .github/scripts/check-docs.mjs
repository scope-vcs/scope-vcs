import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { repositoryFiles } from './repository-files.mjs'

const documentExtensions = new Set(['.adoc', '.markdown', '.md', '.mdx', '.rst'])
const agentInstructions = new Set(['AGENTS.md', 'CLAUDE.md'])
const repositoryHomepage = new Set(['README.html', 'README.md'])
const publishedNotices = new Set(['.github/SECURITY.md', 'media-worker/CODEC-NOTICE.md'])
const permittedPrefixes = ['.agents/skills/', 'legal/']

export function isDocument(file) {
  const normalized = file.replaceAll('\\', '/')
  if (agentInstructions.has(normalized)) return false
  if (repositoryHomepage.has(normalized)) return false
  if (publishedNotices.has(normalized)) return false
  if (permittedPrefixes.some((prefix) => normalized.startsWith(prefix))) return false
  const segments = normalized.split('/')
  if (segments[0] === 'docs') return true
  if (segments.includes('fixtures')) return false
  const basename = segments.at(-1)
  return documentExtensions.has(path.posix.extname(basename).toLowerCase())
    || /^readme(\.|$)/i.test(basename)
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const documents = (await repositoryFiles(root)).filter(isDocument)
  if (documents.length > 0) {
    console.error('Documentation guardrail failed. AGENTS.md: requests and pull requests are the record; do not add documentation files.')
    for (const file of documents) console.error(`  ${file}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
