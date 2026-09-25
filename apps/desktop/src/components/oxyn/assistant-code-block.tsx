import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { CodeIcon } from "@hugeicons/core-free-icons"

import { isSqlBlock } from "@/components/oxyn/assistant-markdown-model"
import type { CodeBlock as CodeBlockNode } from "@/components/oxyn/assistant-markdown-model"
import { ActionMenuContent } from "@/components/oxyn/action-menu-items"
import { AssistantCopyButton } from "@/components/oxyn/assistant-copy-button"
import { copyFromMenu } from "@/components/oxyn/assistant-menu-copy"
import {
  HIGHLIGHT_MAX_CHARS,
  cachedTokens,
  grammarOf,
  highlight,
} from "@/components/oxyn/code-highlight"
import type { CodeLines } from "@/components/oxyn/code-highlight"
import { Button } from "@/components/ui/button"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"

/**
 * The tokens of a closed block, once computed. A block still streaming stays
 * plain text: colouring it at every delta would redo the work for each word,
 * and flicker as a half-written string turns into a keyword.
 */
function useTokens(block: CodeBlockNode): CodeLines | null {
  // An unlabelled fence is read as SQL, as `isSqlBlock` does: this agent
  // answers with queries.
  const grammar =
    block.closed && block.text.length <= HIGHLIGHT_MAX_CHARS
      ? grammarOf(block.language === "" ? "sql" : block.language)
      : null
  const [computed, setComputed] = React.useState<{
    text: string
    lines: CodeLines | null
  } | null>(null)

  React.useEffect(() => {
    if (grammar === null || cachedTokens(grammar, block.text)) return
    let live = true
    void highlight(grammar, block.text).then((lines) => {
      if (live) setComputed({ text: block.text, lines })
    })
    return () => {
      live = false
    }
  }, [grammar, block.text])

  if (grammar === null) return null
  return (
    cachedTokens(grammar, block.text) ??
    (computed?.text === block.text ? computed.lines : null)
  )
}

function Tokens({ lines }: { lines: CodeLines }) {
  return (
    <>
      {lines.map((line, lineIndex) => (
        <React.Fragment key={lineIndex}>
          {lineIndex > 0 ? "\n" : null}
          {line.map((token, tokenIndex) => (
            // The colour comes from Oxyn's theme, the content from the model:
            // React text either way.
            <span
              key={tokenIndex}
              style={{
                color: token.color,
                fontStyle: token.italic ? "italic" : undefined,
              }}
            >
              {token.content}
            </span>
          ))}
        </React.Fragment>
      ))}
    </>
  )
}

export function CodeBlock({
  block,
  onOpenSql,
  openSqlDisabledReason,
  onCopy,
}: {
  block: CodeBlockNode
  onOpenSql?: (sql: string) => void
  openSqlDisabledReason?: string | null
  onCopy?: (text: string) => Promise<boolean> | boolean
}) {
  const sql = onOpenSql !== undefined && isSqlBlock(block)
  const lines = useTokens(block)
  const [anchor, setAnchor] = React.useState<Element | null>(null)
  const code = block.text.trim()
  // The entries of the buttons below, and only those: never `Run` (I-07).
  // Absent from a block that is not SQL; greyed, with the button's reason,
  // where the button is disabled.
  const openSql = !isSqlBlock(block)
    ? "absent"
    : openSqlDisabledReason
      ? { reason: openSqlDisabledReason }
      : true
  const openInConsole = sql ? () => onOpenSql(code) : undefined
  const copyCode = onCopy
    ? () => void copyFromMenu(onCopy, code, "Code")
    : undefined
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <figure
            data-slot="assistant-code"
            className="flex min-w-0 flex-col overflow-hidden rounded-lg border bg-card"
            onContextMenu={(event) => setAnchor(event.target as Element)}
          />
        }
      >
        {block.language !== "" || sql || onCopy ? (
          <figcaption className="flex h-8 items-center justify-between gap-2 border-b px-2.5 text-xs text-muted-foreground">
            <span className="font-mono">
              {block.language || (sql ? "sql" : "code")}
            </span>
            {sql ? (
              <Button
                size="xs"
                variant="ghost"
                disabled={Boolean(openSqlDisabledReason)}
                title={openSqlDisabledReason ?? undefined}
                onClick={() => onOpenSql(code)}
              >
                <HugeiconsIcon
                  icon={CodeIcon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Open in console
              </Button>
            ) : null}
            {onCopy ? (
              <AssistantCopyButton
                text={code}
                label="Copy code"
                onCopy={onCopy}
              />
            ) : null}
          </figcaption>
        ) : null}
        <pre
          data-selectable
          data-highlighted={lines !== null || undefined}
          tabIndex={0}
          aria-label={sql ? "Proposed SQL" : "Code"}
          className="overflow-x-auto p-3 font-mono text-xs leading-5 whitespace-pre outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <code>{lines ? <Tokens lines={lines} /> : block.text}</code>
        </pre>
      </ContextMenuTrigger>
      <ActionMenuContent
        surface="assistantCode"
        anchor={anchor}
        sources={{
          assistant: {
            state: { answering: false, openSql },
            actions: { copyCode, openInConsole },
          },
        }}
      />
    </ContextMenu>
  )
}
