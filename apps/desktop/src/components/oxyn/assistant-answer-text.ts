import { parseMarkdown } from "@/components/oxyn/assistant-markdown-model"
import type { Block, Inline } from "@/components/oxyn/assistant-markdown-model"

// `Copy answer` copies what the answer reads as; `Copy as Markdown` copies its
// source. Both come from the same tree the renderer draws, so the text copied
// is the text shown: a link keeps its address beside its label, an image is
// its alternative text, as on screen.

function inlineText(nodes: Array<Inline>): string {
  return nodes
    .map((node) => {
      switch (node.type) {
        case "text":
        case "code":
          return node.text
        case "strong":
        case "emphasis":
        case "deleted":
          return inlineText(node.children)
        case "link": {
          const label = inlineText(node.children)
          return node.href === "" ? label : `${label} (${node.href})`
        }
        case "image":
          return node.alt.trim() === "" ? "[image]" : `[image: ${node.alt}]`
      }
    })
    .join("")
}

function blockText(block: Block): string {
  switch (block.type) {
    case "paragraph":
    case "heading":
      return inlineText(block.content)
    case "code":
      return block.text.replace(/\n$/, "")
    case "list":
      return block.items
        .map((item, index) =>
          block.ordered
            ? `${block.start + index}. ${inlineText(item)}`
            : `- ${inlineText(item)}`
        )
        .join("\n")
    case "quote":
      return block.blocks.map(blockText).join("\n\n")
    case "rule":
      return ""
    case "table":
      // Tab-separated: pasted into a spreadsheet, it lands in cells.
      return [block.header, ...block.rows]
        .map((row) => row.map(inlineText).join("\t"))
        .join("\n")
  }
}

/** An answer as it reads on screen, without its Markdown markup. */
export function answerReadableText(markdown: string): string {
  return parseMarkdown(markdown)
    .map(blockText)
    .filter((text) => text !== "")
    .join("\n\n")
}
