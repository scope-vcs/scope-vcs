export const REQUEST_MERMAID_MAX_SOURCE_LENGTH = 20_000
export const REQUEST_MERMAID_MAX_EDGES = 200
const MAX_SYNTAX_ITEMS = 400

type BlockedSyntax = {
  message: string
  pattern: RegExp
}

const BLOCKED_SYNTAX: BlockedSyntax[] = [
  {
    message: 'Mermaid configuration frontmatter is not supported.',
    pattern: /^\s*---[ \t]*(?:\r?\n|$)/u,
  },
  {
    message: 'Mermaid directives are not supported.',
    pattern: /%%\s*\{/iu,
  },
  {
    message: 'Mermaid links are not supported.',
    pattern: /(?:^|[;\n])\s*(?:click|link)\s+/iu,
  },
  {
    message: 'Mermaid image and icon syntax is not supported.',
    pattern: /(?:\b(?:img|icon)["']?\s*:|\bicon\s*\(|!\[[^\]]*\]\s*\()/iu,
  },
  {
    // Mermaid parses node metadata as YAML, including escaped property names.
    message: 'Escapes in Mermaid node metadata are not supported.',
    pattern: /@\s*\{[^}]*\\/u,
  },
  {
    message: 'HTML and SVG markup is not supported in Mermaid source.',
    pattern: /<\s*\/?\s*(?:a|audio|embed|foreignobject|iframe|image|img|link|meta|object|script|style|use|video)\b/iu,
  },
  {
    message: 'External resource attributes are not supported in Mermaid source.',
    pattern: /\b(?:href|src|xlink:href)\s*[:=]/iu,
  },
  {
    message: 'External resource URLs are not supported in Mermaid source.',
    pattern: /\b(?:blob|data|file|ftp|https?|javascript):/iu,
  },
  {
    message: 'CSS resource loading is not supported in Mermaid source.',
    pattern: /(?:\burl\s*\(|@import\b|@font-face\b)/iu,
  },
  {
    message: 'Custom Mermaid styling is not supported.',
    pattern: /(?:^|[;\n])\s*(?:classDef|linkStyle|style)\b/iu,
  },
]

export function assertRequestMermaidSource(source: string) {
  if (!source.trim()) throw new Error('Mermaid source cannot be empty.')
  if (source.length > REQUEST_MERMAID_MAX_SOURCE_LENGTH) {
    throw new Error(
      `Mermaid source cannot exceed ${REQUEST_MERMAID_MAX_SOURCE_LENGTH.toLocaleString('en-US')} characters.`,
    )
  }
  for (const blocked of BLOCKED_SYNTAX) {
    if (blocked.pattern.test(source)) throw new Error(blocked.message)
  }
  // Count labels and numeric values too: edge limits alone miss disconnected
  // nodes, actors, and chart data. This conservative budget runs before parsing.
  const items = source.match(/[^\s;,\[\]{}()<>|:&=+\-]+/gu) ?? []
  if (items.length > MAX_SYNTAX_ITEMS) {
    throw new Error('This diagram is too complex to render here. Split it into smaller diagrams.')
  }
}
