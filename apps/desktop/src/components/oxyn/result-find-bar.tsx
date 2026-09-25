import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  ArrowUp01Icon,
  Cancel01Icon,
  Search01Icon,
} from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
} from "@/components/ui/input-group"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { Spinner } from "@/components/ui/spinner"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { FindAnswer } from "@/lib/ipc/results"
import { cn } from "@/lib/utils"
import { InputGroupTextInput } from "./text-field"

export type FindDirection = "first" | "next" | "previous"

/**
 * What a search found, in words. Pure, so it is tested without a DOM.
 *
 * What was **not** searched is said with it: « no match » over batches that
 * spilled to disk would read as « not in this result » (oxyn-data `find`).
 */
export function findSummary(answer: FindAnswer): {
  text: string
  partial: boolean
} {
  let text =
    answer.total === 0
      ? "No match"
      : answer.total === 1
        ? "1 matching row"
        : `${answer.total.toLocaleString("en-US")} matching rows`
  if (answer.total > 0 && answer.ordinal !== null) {
    text = `${answer.ordinal.toLocaleString("en-US")} of ${text}`
  }
  if (answer.capped) text += " or more · the search stopped counting"
  if (answer.skippedBatches > 0) {
    text += ` · ${answer.skippedBatches.toLocaleString("en-US")} ${answer.skippedBatches === 1 ? "batch" : "batches"} not searched: they spilled to disk`
  }
  return { text, partial: answer.capped || answer.skippedBatches > 0 }
}

/**
 * `Find in loaded results…`: reveals where the text is, hides nothing.
 *
 * A search is not a filter — every row stays in the grid and in the export
 * (docs/UX-SPEC.md, « Ce qui est exporté est ce qui est affiché »). Enter finds
 * the next match, Shift+Enter the previous one, Escape clears.
 */
export function ResultFindBar({
  answer,
  searching,
  error,
  onFind,
  onClear,
  className,
}: {
  answer: FindAnswer | null
  searching: boolean
  error: string | null
  onFind: (needle: string, direction: FindDirection) => void
  onClear: () => void
  className?: string
}) {
  const [needle, setNeedle] = React.useState("")
  const [searched, setSearched] = React.useState<string | null>(null)

  const find = (direction: "next" | "previous") => {
    if (needle.trim() === "") return
    if (needle !== searched) {
      setSearched(needle)
      onFind(needle, "first")
    } else {
      onFind(needle, direction)
    }
  }

  const clear = () => {
    setNeedle("")
    setSearched(null)
    onClear()
  }

  const summary = answer && searched !== null ? findSummary(answer) : null
  const canStep = summary !== null && (answer?.total ?? 0) > 0
  const status =
    error ??
    summary?.text ??
    "Searches the rows already loaded; nothing is hidden."

  return (
    <div
      className={cn(
        "flex h-10 min-w-0 shrink-0 items-center gap-2 border-b px-2",
        className
      )}
    >
      <InputGroup className="h-8 max-w-80 min-w-32 shrink">
        <InputGroupAddon>
          {searching ? (
            <Spinner />
          ) : (
            <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
          )}
        </InputGroupAddon>
        <InputGroupTextInput
          value={needle}
          onChange={(event) => setNeedle(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault()
              find(event.shiftKey ? "previous" : "next")
            } else if (event.key === "Escape") {
              event.preventDefault()
              clear()
            }
          }}
          placeholder="Find in loaded results…"
          aria-label="Find in loaded results"
        />
        {needle !== "" ? (
          <InputGroupAddon align="inline-end">
            <InputGroupButton
              size="icon-xs"
              aria-label="Clear search"
              onClick={clear}
            >
              <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
            </InputGroupButton>
          </InputGroupAddon>
        ) : null}
      </InputGroup>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label="Previous match"
              disabled={!canStep || searching}
              onClick={() => find("previous")}
            />
          }
        >
          <HugeiconsIcon icon={ArrowUp01Icon} strokeWidth={2} />
        </TooltipTrigger>
        <TooltipContent>
          Previous match
          <KbdGroup>
            <Kbd>⇧</Kbd>
            <Kbd>↵</Kbd>
          </KbdGroup>
        </TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label="Next match"
              disabled={!canStep || searching}
              onClick={() => find("next")}
            />
          }
        >
          <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
        </TooltipTrigger>
        <TooltipContent>
          Next match
          <Kbd>↵</Kbd>
        </TooltipContent>
      </Tooltip>
      <p
        role="status"
        // « batches not searched: they spilled to disk » is the part a narrow
        // bar cuts first; it stays reachable rather than disappearing.
        title={status}
        className={cn(
          "min-w-0 truncate text-xs",
          error || summary?.partial ? "text-warning" : "text-muted-foreground"
        )}
      >
        {status}
      </p>
    </div>
  )
}
