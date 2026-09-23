// Syntax colouring for the code blocks of an answer, as tokens — never HTML.
//
// shiki can return HTML (`codeToHtml`); Oxyn asks for tokens instead and draws
// each one as a React `<span>`, so the text of a block stays text whatever the
// model wrote (docs/SECURITY.md, « Surface d'entrée »). The regex engine is
// shiki's JavaScript one: the Oniguruma engine is WebAssembly, which the
// production CSP (`script-src 'self'`, no `wasm-unsafe-eval`) refuses.
//
// Everything here loads on demand: shiki, its engine and each grammar are
// fetched the first time a closed block needs them, never at start-up.

import type { HighlighterCore, ThemeRegistration } from "shiki/core"

/** A run of text and how to draw it. `color` is a CSS value from the theme. */
export interface CodeToken {
  content: string
  color?: string
  italic: boolean
}

export type CodeLines = Array<Array<CodeToken>>

/** Beyond this a block is shown as text: tokenising runs on the UI thread. */
export const HIGHLIGHT_MAX_CHARS = 50_000

const GRAMMARS = {
  sql: () => import("shiki/langs/sql.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  typescript: () => import("shiki/langs/typescript.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  shellscript: () => import("shiki/langs/shellscript.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
  diff: () => import("shiki/langs/diff.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  go: () => import("shiki/langs/go.mjs"),
} as const

type Grammar = keyof typeof GRAMMARS

/** What a fence may say, to the grammar that reads it. */
const ALIASES: Record<string, Grammar> = {
  sql: "sql",
  postgres: "sql",
  postgresql: "sql",
  pgsql: "sql",
  psql: "sql",
  mysql: "sql",
  sqlite: "sql",
  plsql: "sql",
  tsql: "sql",
  json: "json",
  jsonc: "json",
  json5: "json",
  js: "javascript",
  javascript: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  ts: "typescript",
  typescript: "typescript",
  jsx: "jsx",
  tsx: "tsx",
  py: "python",
  python: "python",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  shell: "shellscript",
  shellscript: "shellscript",
  console: "shellscript",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  xml: "xml",
  html: "xml",
  svg: "xml",
  diff: "diff",
  patch: "diff",
  rust: "rust",
  rs: "rust",
  go: "go",
  golang: "go",
}

/** The grammar a fence names, or `null` for a language not coloured. */
export function grammarOf(language: string): Grammar | null {
  const key = language.toLowerCase()
  return Object.hasOwn(ALIASES, key) ? (ALIASES[key] ?? null) : null
}

/**
 * The SQL editor's palette (`sql-editor.tsx`), as CSS variables: the tokens
 * follow the app's theme, light or dark, without a second theme to load.
 */
const THEME: ThemeRegistration = {
  name: "oxyn",
  type: "dark",
  colors: {
    "editor.foreground": "var(--foreground)",
    "editor.background": "var(--card)",
  },
  tokenColors: [
    {
      scope: ["comment", "punctuation.definition.comment"],
      settings: { foreground: "var(--muted-foreground)", fontStyle: "italic" },
    },
    {
      scope: [
        "keyword",
        "storage.modifier",
        "storage.type.function",
        "storage.type.class",
        "keyword.other",
        "keyword.operator.word",
        "keyword.control",
      ],
      settings: { foreground: "var(--primary-text)" },
    },
    {
      scope: ["string", "punctuation.definition.string", "string.regexp"],
      settings: { foreground: "var(--success)" },
    },
    {
      scope: [
        "constant.numeric",
        "constant.language",
        "constant.character",
        "keyword.other.unit",
      ],
      settings: { foreground: "var(--warning)" },
    },
    {
      scope: [
        "storage.type",
        "entity.name.type",
        "support.type",
        "support.class",
        "entity.name.tag",
        "support.type.property-name",
      ],
      settings: { foreground: "var(--chart-4)" },
    },
    {
      scope: ["keyword.operator", "punctuation", "meta.brace"],
      settings: { foreground: "var(--muted-foreground)" },
    },
  ],
}

let highlighter: Promise<HighlighterCore> | null = null

function loadHighlighter() {
  highlighter ??= Promise.all([
    import("shiki/core"),
    import("shiki/engine/javascript"),
  ]).then(([core, engine]) =>
    core.createHighlighterCore({
      themes: [THEME],
      langs: [],
      // `auto` uses the RegExp `v` flag only where the engine has it: macOS 13
      // ships a WebKit without it, and falls back to ES2018 patterns.
      engine: engine.createJavaScriptRegexEngine({ forgiving: true }),
    })
  )
  return highlighter
}

const loading = new Map<Grammar, Promise<void>>()

async function ensureGrammar(core: HighlighterCore, grammar: Grammar) {
  let pending = loading.get(grammar)
  if (!pending) {
    pending = GRAMMARS[grammar]().then((module) =>
      core.loadLanguage(module.default)
    )
    loading.set(grammar, pending)
  }
  await pending
}

/** Coloured blocks, by grammar and text: a block streams once, then stays. */
const CACHE_SIZE = 200
const cache = new Map<string, CodeLines>()

function cacheKey(grammar: Grammar, text: string) {
  return JSON.stringify([grammar, text])
}

/** The tokens already computed for this block, without waiting. */
export function cachedTokens(grammar: Grammar, text: string) {
  return cache.get(cacheKey(grammar, text)) ?? null
}

/** Tokenises a block. Resolves to `null` when the grammar cannot be read. */
export async function highlight(
  grammar: Grammar,
  text: string
): Promise<CodeLines | null> {
  const key = cacheKey(grammar, text)
  const hit = cache.get(key)
  if (hit) return hit
  try {
    const core = await loadHighlighter()
    await ensureGrammar(core, grammar)
    const result = core.codeToTokens(text, { lang: grammar, theme: "oxyn" })
    const lines: CodeLines = result.tokens.map((line) =>
      line.map((token) => ({
        content: token.content,
        color: token.color,
        // shiki's `FontStyle.Italic` is the bit 1.
        italic: ((token.fontStyle ?? 0) & 1) === 1,
      }))
    )
    if (cache.size >= CACHE_SIZE) {
      const oldest = cache.keys().next()
      if (!oldest.done) cache.delete(oldest.value)
    }
    cache.set(key, lines)
    return lines
  } catch {
    // Colouring is a comfort: a grammar that fails leaves the text readable.
    return null
  }
}
