// A mermaid block of an answer, turned into an image — never into DOM.
//
// The source is the model's, so it is hostile input (docs/SECURITY.md). Three
// locks, each enough on its own for what it covers:
//
// 1. mermaid runs with `securityLevel: "strict"` (its output goes through its
//    DOMPurify pass, clicks and links are dropped) and `htmlLabels: false`
//    (labels are SVG text, not `foreignObject` HTML). Those keys, and every
//    other one that could loosen them, are listed in `secure`: mermaid refuses
//    to let a diagram's own configuration change them.
// 2. Before mermaid sees it, the source loses its `%%{init}%%` directives and
//    its front-matter: a diagram has no say in how it is rendered.
// 3. The SVG is shown through `<img src="data:image/svg+xml;base64,…">`. An
//    image document runs no script, loads nothing and receives no event, so
//    whatever survived the first two locks stays inert.
//
// mermaid itself measures text in a hidden element of the document while it
// lays out; that element is its own, sanitised, and removed when it is done.

/** Characters mermaid accepts, and the bound checked before loading it. */
export const MERMAID_MAX_TEXT = 20_000

/** Edges one diagram may draw. */
const MERMAID_MAX_EDGES = 500

const DIRECTIVE = /%%\{[\s\S]*?\}%%/g
const FRONT_MATTER = /^\s*---\r?\n[\s\S]*?\r?\n---[ \t]*(?:\r?\n|$)/

/**
 * The source without what would configure its own rendering. `stripped` says
 * whether anything was removed, so the block can tell the user.
 */
export function withoutConfiguration(source: string): {
  text: string
  stripped: boolean
} {
  const text = source.replace(FRONT_MATTER, "").replace(DIRECTIVE, "")
  return { text, stripped: text !== source }
}

/** UTF-8, then base64: `btoa` alone throws on anything past Latin-1. */
export function svgDataUrl(svg: string): string {
  const bytes = new TextEncoder().encode(svg)
  let binary = ""
  // In slices: spreading a whole diagram into `fromCharCode` overflows the
  // argument limit.
  for (let index = 0; index < bytes.length; index += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
  return `data:image/svg+xml;base64,${btoa(binary)}`
}

/** The drawing's own size, from its `viewBox`: an image has no other. */
export function svgSize(svg: string): { width: number; height: number } | null {
  const root = /<svg\b[^>]*>/i.exec(svg)?.[0] ?? ""
  const box = /viewBox="([^"]*)"/i.exec(root)?.[1]
  if (!box) return null
  const [, , width, height] = box
    .trim()
    .split(/[\s,]+/)
    .map(Number)
  if (!width || !height || !Number.isFinite(width) || !Number.isFinite(height))
    return null
  return { width, height }
}

export type MermaidOutcome =
  | {
      status: "drawn"
      src: string
      width: number
      height: number
      stripped: boolean
    }
  | { status: "invalid"; message: string; stripped: boolean }

// mermaid's configuration is global: two diagrams rendering at once in two
// themes would each read the other's. One at a time.
let queue: Promise<unknown> = Promise.resolve()
let sequence = 0

async function draw(source: string, dark: boolean): Promise<MermaidOutcome> {
  const { text, stripped } = withoutConfiguration(source)
  if (text.length > MERMAID_MAX_TEXT)
    return {
      status: "invalid",
      message: `The diagram is ${text.length} characters long; Oxyn draws up to ${MERMAID_MAX_TEXT}.`,
      stripped,
    }
  const { default: mermaid } = await import("mermaid")
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: "strict",
    htmlLabels: false,
    flowchart: { htmlLabels: false },
    maxTextSize: MERMAID_MAX_TEXT,
    maxEdges: MERMAID_MAX_EDGES,
    suppressErrorRendering: true,
    theme: dark ? "dark" : "neutral",
    // The image cannot reach the app's fonts: measure and draw with the
    // system's, or labels are measured in one font and drawn in another.
    fontFamily: "system-ui, -apple-system, sans-serif",
    secure: [
      "secure",
      "securityLevel",
      "startOnLoad",
      "maxTextSize",
      "maxEdges",
      "suppressErrorRendering",
      "htmlLabels",
      "flowchart",
      "dompurifyConfig",
      "theme",
      "themeCSS",
      "themeVariables",
      "fontFamily",
      "altFontFamily",
    ],
  })
  sequence += 1
  const id = `oxyn-mermaid-${sequence}`
  try {
    await mermaid.parse(text)
    const { svg } = await mermaid.render(id, text)
    const size = svgSize(svg) ?? { width: 600, height: 400 }
    return { status: "drawn", src: svgDataUrl(svg), ...size, stripped }
  } catch (error) {
    return {
      status: "invalid",
      message: error instanceof Error ? error.message : String(error),
      stripped,
    }
  } finally {
    // A failed render may leave its measuring element behind.
    document.getElementById(`d${id}`)?.remove()
    document.getElementById(id)?.remove()
  }
}

/** Draws a closed mermaid block. Never rejects: a failure is an outcome. */
export function renderMermaid(
  source: string,
  dark: boolean
): Promise<MermaidOutcome> {
  const next = queue.then(() => draw(source, dark))
  queue = next.catch(() => undefined)
  return next.catch((error: unknown) => ({
    status: "invalid" as const,
    message: error instanceof Error ? error.message : String(error),
    stripped: false,
  }))
}
