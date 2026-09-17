import { HugeiconsIcon } from "@hugeicons/react"
import { RepeatIcon } from "@hugeicons/core-free-icons"

import { AssistantCopyButton } from "@/components/oxyn/assistant-copy-button"
import { Button } from "@/components/ui/button"
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
      <AssistantCopyButton text={text} label="Copy answer" onCopy={onCopy} />
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
