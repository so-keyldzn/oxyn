import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { RepeatIcon } from "@hugeicons/core-free-icons"

import { ActionMenuContent } from "@/components/oxyn/action-menu-items"
import { answerReadableText } from "@/components/oxyn/assistant-answer-text"
import { AssistantCopyButton } from "@/components/oxyn/assistant-copy-button"
import { copyFromMenu } from "@/components/oxyn/assistant-menu-copy"
import { Button } from "@/components/ui/button"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

/**
 * Copy and regenerate, under a finished answer.
 *
 * Regenerating asks the same question again as another version. It repeats the
 * model call, never a tool call: a tool runs again only if the model asks for
 * it, through the same gate (I-13).
 *
 * `text` is the answer's Markdown; `Copy answer` copies it as it reads, the
 * same text the context menu's entry of that name copies.
 */
export function AssistantAnswerActions({
  text,
  canRegenerate,
  onRegenerate,
  onCopy,
}: {
  text: string
  canRegenerate: boolean
  onRegenerate: () => void
  onCopy: (text: string) => Promise<boolean> | boolean
}) {
  return (
    <div
      data-slot="assistant-answer-actions"
      role="group"
      aria-label="Answer actions"
      className="flex items-center gap-0.5"
    >
      <AssistantCopyButton
        text={answerReadableText(text)}
        label="Copy answer"
        onCopy={onCopy}
      />
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              type="button"
              size="icon-xs"
              variant="ghost"
              aria-label="Regenerate answer"
              disabled={!canRegenerate}
              onClick={onRegenerate}
            />
          }
        >
          <HugeiconsIcon icon={RepeatIcon} strokeWidth={2} />
        </TooltipTrigger>
        <TooltipContent>Ask again, as another version</TooltipContent>
      </Tooltip>
    </div>
  )
}

/**
 * The context menu of an answer: `Copy answer`, `Copy as Markdown`,
 * `Regenerate answer` — the callbacks of the buttons above (I-01).
 *
 * `answering` greys `Regenerate answer`, as the button is disabled; without
 * `onRegenerate` the entry is absent.
 */
export function AssistantAnswerMenu({
  text,
  answering,
  onCopy,
  onRegenerate,
  children,
}: {
  /** The answer's Markdown. */
  text: string
  answering: boolean
  onCopy: (text: string) => Promise<boolean> | boolean
  onRegenerate?: () => void
  children: React.ReactNode
}) {
  const [anchor, setAnchor] = React.useState<Element | null>(null)
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div
            data-slot="assistant-answer"
            onContextMenu={(event) => setAnchor(event.target as Element)}
          />
        }
      >
        {children}
      </ContextMenuTrigger>
      <ActionMenuContent
        surface="assistantAnswer"
        anchor={anchor}
        sources={{
          assistant: {
            state: { answering },
            actions: {
              copyAnswer: () =>
                void copyFromMenu(onCopy, answerReadableText(text), "Answer"),
              copyMarkdown: () => void copyFromMenu(onCopy, text, "Markdown"),
              regenerate: onRegenerate,
            },
          },
        }}
      />
    </ContextMenu>
  )
}
