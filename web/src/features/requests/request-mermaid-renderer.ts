import type { DOMPurify } from 'dompurify'
import {
  assertRequestMermaidSource,
  REQUEST_MERMAID_MAX_EDGES,
  REQUEST_MERMAID_MAX_SOURCE_LENGTH,
} from './request-mermaid-policy'

export type MermaidRenderInput = {
  source: string
  theme: 'light' | 'dark'
}

export type MermaidRenderResult = {
  svg: string
  width: number
  height: number
}

type MermaidModule = typeof import('mermaid')

type RendererDependencies = {
  DOMPurify: DOMPurify
  mermaid: MermaidModule['default']
}

const SYSTEM_FONT = 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
const SAFE_FRAGMENT_REFERENCE = /^#[A-Za-z0-9_][\w:.-]*$/u
const CSS_URL = /url\(\s*(['"]?)(.*?)\1\s*\)/giu
const EXTERNAL_CSS = /@(?:font-face|import)\b/iu
const FORBIDDEN_RESOURCE_ELEMENTS = 'a,audio,embed,foreignobject,iframe,image,img,link,meta,object,script,video'
const FORBIDDEN_RESOURCE_SELECTOR = `${FORBIDDEN_RESOURCE_ELEMENTS},foreignObject`

let dependenciesPromise: Promise<RendererDependencies> | null = null
let renderSequence = 0

function loadRendererDependencies() {
  if (!dependenciesPromise) {
    dependenciesPromise = Promise.all([
      import('mermaid'),
      import('dompurify') as Promise<{ default: DOMPurify }>,
    ]).then(([mermaidModule, domPurifyModule]) => ({
      mermaid: mermaidModule.default,
      DOMPurify: domPurifyModule.default,
    })).catch((error: unknown) => {
      dependenciesPromise = null
      throw error
    })
  }
  return dependenciesPromise
}

function isSafeCss(css: string) {
  if (EXTERNAL_CSS.test(css)) return false
  CSS_URL.lastIndex = 0
  for (const match of css.matchAll(CSS_URL)) {
    if (!SAFE_FRAGMENT_REFERENCE.test(match[2]?.trim() ?? '')) return false
  }
  return true
}

function sanitizeSvg(DOMPurify: RendererDependencies['DOMPurify'], svg: string) {
  const sanitized = String(DOMPurify.sanitize(svg, {
    ADD_ATTR: [
      'dominant-baseline',
      'marker-end',
      'marker-mid',
      'marker-start',
      'xlink:href',
    ],
    ADD_TAGS: ['style', 'use'],
    ALLOW_UNKNOWN_PROTOCOLS: false,
    FORBID_TAGS: FORBIDDEN_RESOURCE_ELEMENTS.split(','),
    USE_PROFILES: { svg: true, svgFilters: true },
  }))
  const document = new DOMParser().parseFromString(sanitized, 'image/svg+xml')
  const parserError = document.querySelector('parsererror')
  const root = document.documentElement
  if (parserError || root.localName !== 'svg') throw new Error('Mermaid produced invalid SVG.')
  if (root.querySelector(FORBIDDEN_RESOURCE_SELECTOR)) {
    throw new Error('Mermaid produced an unsupported external resource.')
  }
  for (const element of root.querySelectorAll('*')) {
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase()
      if (name.startsWith('on')) throw new Error('Mermaid produced an unsafe SVG event handler.')
      if (name === 'href' || name === 'xlink:href') {
        if (!SAFE_FRAGMENT_REFERENCE.test(attribute.value.trim())) {
          throw new Error('Mermaid produced an external SVG reference.')
        }
      }
      if (name === 'src') throw new Error('Mermaid produced an external SVG resource.')
      if (name === 'style' && !isSafeCss(attribute.value)) {
        throw new Error('Mermaid produced an external CSS resource.')
      }
    }
  }
  for (const style of root.querySelectorAll('style')) {
    if (!isSafeCss(style.textContent ?? '')) {
      throw new Error('Mermaid produced an external CSS resource.')
    }
  }
  return { root, svg: sanitized }
}

function viewBoxSize(root: Element) {
  const parts = root.getAttribute('viewBox')?.trim().split(/[\s,]+/u).map(Number)
  if (!parts || parts.length !== 4 || parts.some((part) => !Number.isFinite(part))) {
    throw new Error('Mermaid SVG is missing a valid viewBox.')
  }
  const width = parts[2]
  const height = parts[3]
  if (!width || width <= 0 || !height || height <= 0) {
    throw new Error('Mermaid SVG has invalid dimensions.')
  }
  return { width, height }
}

async function render(input: MermaidRenderInput): Promise<MermaidRenderResult> {
  if (typeof document === 'undefined' || typeof DOMParser === 'undefined') {
    throw new Error('Mermaid rendering requires a browser DOM.')
  }
  const { DOMPurify, mermaid } = await loadRendererDependencies()
  mermaid.initialize({
    deterministicIds: true,
    fontFamily: SYSTEM_FONT,
    htmlLabels: false,
    layout: 'dagre',
    look: 'classic',
    maxEdges: REQUEST_MERMAID_MAX_EDGES,
    maxTextSize: REQUEST_MERMAID_MAX_SOURCE_LENGTH,
    secure: [
      'secure',
      'securityLevel',
      'startOnLoad',
      'maxTextSize',
      'maxEdges',
      'suppressErrorRendering',
      'htmlLabels',
      'layout',
      'look',
      'theme',
      'fontFamily',
    ],
    securityLevel: 'strict',
    startOnLoad: false,
    suppressErrorRendering: true,
    theme: input.theme === 'dark' ? 'dark' : 'default',
  })
  const id = `request-mermaid-${++renderSequence}`
  try {
    const rendered = await mermaid.render(id, input.source)
    const sanitized = sanitizeSvg(DOMPurify, rendered.svg)
    return { ...viewBoxSize(sanitized.root), svg: sanitized.svg }
  } finally {
    document.getElementById(id)?.remove()
    document.getElementById(`d${id}`)?.remove()
    document.getElementById(`i${id}`)?.remove()
  }
}

export async function renderRequestMermaid(input: MermaidRenderInput): Promise<MermaidRenderResult> {
  assertRequestMermaidSource(input.source)
  return render(input)
}
