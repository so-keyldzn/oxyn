import * as React from "react"

import { CodeBlock } from "@/components/oxyn/assistant-code-block"
import {
  isErdBlock,
  isMermaidBlock,
  parseErdNames,
  parseMarkdown,
} from "@/components/oxyn/assistant-markdown-model"
import type {
  Block,
  ErdRequest,
  Inline,
} from "@/components/oxyn/assistant-markdown-model"
import { AssistantMermaid } from "@/components/oxyn/assistant-mermaid"
import { Separator } from "@/components/ui/separator"
import { cn } from "@/lib/utils"

function InlineContent({ nodes }: { nodes: Array<Inline> }) {
  return (
    <>
      {nodes.map((node, index) => {
        switch (node.type) {
          case "text":
            return <React.Fragment key={index}>{node.text}</React.Fragment>
          case "code":
            return (
              <code
                key={index}
                className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]"
              >
                {node.text}
              </code>
            )
          case "strong":
            return (
              <strong key={index} className="font-semibold">
                <InlineContent nodes={node.children} />
              </strong>
            )
          case "emphasis":
            return (
              <em key={index}>
                <InlineContent nodes={node.children} />
              </em>
            )
          case "deleted":
            return (
              <del key={index}>
                <InlineContent nodes={node.children} />
              </del>
            )
          case "link":
            // Neutralized: the model chooses the address, so nothing here
            // navigates, opens or fetches. The address stays readable.
            return (
              <span key={index} data-slot="assistant-link">
                <span className="underline decoration-dotted underline-offset-3">
                  <InlineContent nodes={node.children} />
                </span>
                {node.href !== "" ? (
                  <span className="ml-1 font-mono text-[0.8em] break-all text-muted-foreground">
                    ({node.href})
                  </span>
                ) : null}
              </span>
            )
          case "image":
            // No image ever loads: a remote image is a request the model chose.
            return (
              <span key={index} className="text-muted-foreground">
                [image{node.alt.trim() !== "" ? `: ${node.alt}` : ""}]
              </span>
            )
        }
      })}
    </>
  )
}

function ErdBlock({
  block,
  renderErd,
}: {
  block: Extract<Block, { type: "code" }>
  renderErd: (request: ErdRequest) => React.ReactNode
}) {
  // Parsed once per text: a closed block no longer changes while the rest of
  // the answer streams.
  const request = React.useMemo(() => {
    const { names, dropped } = parseErdNames(block.text)
    return { names, ignoredNames: dropped, source: block.text }
  }, [block.text])
  return <>{renderErd(request)}</>
}

function Blocks({
  blocks,
  onOpenSql,
  openSqlDisabledReason,
  onCopy,
  renderErd,
}: {
  blocks: Array<Block>
  onOpenSql?: (sql: string) => void
  openSqlDisabledReason?: string | null
  onCopy?: (text: string) => Promise<boolean> | boolean
  renderErd?: (request: ErdRequest) => React.ReactNode
}) {
  return (
    <>
      {blocks.map((block, index) => {
        switch (block.type) {
          case "paragraph":
            return (
              <p key={index}>
                <InlineContent nodes={block.content} />
              </p>
            )
          case "heading": {
            const Tag = `h${Math.min(block.level + 2, 6)}` as "h3"
            return (
              <Tag
                key={index}
                className={cn(
                  "font-semibold text-foreground",
                  block.level <= 2 ? "text-base" : "text-sm"
                )}
              >
                <InlineContent nodes={block.content} />
              </Tag>
            )
          }
          case "code":
            // Only once closed: a half-written list would draw, then redraw,
            // a diagram of the tables named so far.
            if (renderErd && isErdBlock(block))
              return (
                <ErdBlock key={index} block={block} renderErd={renderErd} />
              )
            // Closed only, for the same reason; and loaded only then.
            if (isMermaidBlock(block))
              return <AssistantMermaid key={index} source={block.text} />
            return (
              <CodeBlock
                key={index}
                block={block}
                onOpenSql={onOpenSql}
                openSqlDisabledReason={openSqlDisabledReason}
                onCopy={onCopy}
              />
            )
          case "list": {
            const items = block.items.map((item, itemIndex) => (
              <li key={itemIndex}>
                <InlineContent nodes={item} />
              </li>
            ))
            return block.ordered ? (
              <ol
                key={index}
                start={block.start}
                className="flex list-decimal flex-col gap-1 pl-5"
              >
                {items}
              </ol>
            ) : (
              <ul key={index} className="flex list-disc flex-col gap-1 pl-5">
                {items}
              </ul>
            )
          }
          case "quote":
            return (
              <blockquote
                key={index}
                className="flex flex-col gap-2 border-l-2 pl-3 text-muted-foreground"
              >
                <Blocks blocks={block.blocks} renderErd={renderErd} />
              </blockquote>
            )
          case "rule":
            return <Separator key={index} />
          case "table":
            return (
              <div
                key={index}
                tabIndex={0}
                role="region"
                aria-label="Table"
                className="overflow-x-auto rounded-lg border outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <table className="w-full text-xs">
                  <thead className="bg-muted/50">
                    <tr>
                      {block.header.map((cell, cellIndex) => (
                        <th
                          key={cellIndex}
                          className="px-2 py-1.5 text-left font-medium"
                        >
                          <InlineContent nodes={cell} />
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {block.rows.map((row, rowIndex) => (
                      <tr key={rowIndex} className="border-t">
                        {row.map((cell, cellIndex) => (
                          <td key={cellIndex} className="px-2 py-1.5 align-top">
                            <InlineContent nodes={cell} />
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )
        }
      })}
    </>
  )
}

/**
 * A model's answer, drawn from a tree of text.
 *
 * No `dangerouslySetInnerHTML`, no `<a href>`, no `<img>`: a model's output is
 * hostile input (docs/SECURITY.md), and the renderer offers it no element that
 * loads, navigates or runs. A closed SQL block carries « Open in console »,
 * which hands its text to the caller and runs nothing (I-07).
 */
export function AssistantMarkdown({
  text,
  onOpenSql,
  openSqlDisabledReason,
  onCopy,
  renderErd,
  className,
}: {
  text: string
  onOpenSql?: (sql: string) => void
  /** Why opening is unavailable, shown on the disabled button. */
  openSqlDisabledReason?: string | null
  /** Offers to copy each code block; resolves to whether it worked. */
  onCopy?: (text: string) => Promise<boolean> | boolean
  /**
   * Draws a closed `erd` block — the feature resolves its names against the
   * catalog. Without it the block stays code, as any other.
   */
  renderErd?: (request: ErdRequest) => React.ReactNode
  className?: string
}) {
  const blocks = React.useMemo(() => parseMarkdown(text), [text])
  return (
    <div
      data-slot="assistant-markdown"
      className={cn(
        "flex min-w-0 flex-col gap-3 text-sm leading-relaxed wrap-break-word",
        className
      )}
    >
      <Blocks
        blocks={blocks}
        onOpenSql={onOpenSql}
        openSqlDisabledReason={openSqlDisabledReason}
        onCopy={onCopy}
        renderErd={renderErd}
      />
    </div>
  )
}
