import {
  getFiletypeFromFileName,
  parseDiffFromFile,
  type FileDiffOptions,
  type FileOptions,
} from '@pierre/diffs'
import { preloadFile, preloadFileDiff } from '@pierre/diffs/ssr'
import { parentPort } from 'node:worker_threads'
import type {
  CodeRenderDiffInput,
  CodeRenderFileInput,
  CodeRenderWorkerInput,
  CodeRenderWorkerResult,
} from './code-render-contract'

const CODE_OPTIONS = {
  disableFileHeader: true,
  overflow: 'wrap',
  preferredHighlighter: 'highlights',
  tokenizeMaxLineLength: 1_000,
} satisfies FileOptions<undefined, undefined>

const DIFF_OPTIONS = {
  ...CODE_OPTIONS,
  diffStyle: 'unified',
  hunkSeparators: 'line-info-basic',
  lineDiffType: 'word',
  maxLineDiffLength: 512,
} satisfies FileDiffOptions<undefined, undefined>

async function renderDiff(input: CodeRenderDiffInput): Promise<CodeRenderWorkerResult> {
  const lang = codeLanguage(input.path)
  const fileDiff = parseDiffFromFile(
    { contents: input.oldText, lang, name: input.path },
    { contents: input.newText, lang, name: input.path },
  )

  if (fileDiff.hunks.length === 0) {
    return { kind: 'empty' }
  }
  if (fileDiff.hunks.length > input.budget.maxHunks) {
    return { kind: 'omitted', reason: 'hunks' }
  }
  if (fileDiff.unifiedLineCount > input.budget.maxRenderedLines) {
    return { kind: 'omitted', reason: 'lines' }
  }

  const { prerenderedHTML } = await preloadFileDiff({
    fileDiff,
    options: {
      ...DIFF_OPTIONS,
      tokenizeMaxLength: input.budget.maxRenderedLines,
    },
  })
  return boundedHtml(prerenderedHTML, input.budget.maxOutputBytes)
}

async function renderFile(input: CodeRenderFileInput): Promise<CodeRenderWorkerResult> {
  const { prerenderedHTML } = await preloadFile({
    file: { contents: input.text, lang: codeLanguage(input.path), name: input.path },
    options: CODE_OPTIONS,
  })
  return boundedHtml(prerenderedHTML, input.budget.maxOutputBytes)
}

function codeLanguage(path: string) {
  const lowerPath = path.toLowerCase()
  if (lowerPath.endsWith('.sh')) return 'shellscript'
  const language = getFiletypeFromFileName(path)
  return language === 'text' ? getFiletypeFromFileName(lowerPath) : language
}

function boundedHtml(html: string, maxOutputBytes: number): CodeRenderWorkerResult {
  return Buffer.byteLength(html) > maxOutputBytes
    ? { kind: 'omitted', reason: 'output' }
    : { html, kind: 'html' }
}

const port = parentPort
if (!port) {
  throw new Error('The code renderer must run in a worker thread')
}

port.on('message', (input: CodeRenderWorkerInput) => {
  const rendered = input.kind === 'file' ? renderFile(input) : renderDiff(input)
  rendered.then(
    (result) => port.postMessage(result),
    () => port.postMessage({ kind: 'error' } satisfies CodeRenderWorkerResult),
  )
})

port.postMessage({ kind: 'ready' })
