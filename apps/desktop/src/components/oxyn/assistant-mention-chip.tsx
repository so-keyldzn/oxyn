import { HugeiconsIcon } from "@hugeicons/react"
import {
  CustomFieldIcon,
  EyeIcon,
  FileScriptIcon,
  Layers01Icon,
  TableIcon,
} from "@hugeicons/core-free-icons"

import { ActionMenuContent } from "@/components/oxyn/action-menu-items"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import { Button } from "@/components/ui/button"
import { ContentHint } from "./content-hint"
import type { Mention } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

/** What the list shows for an object, and so what its chip shows. */
export type MentionKind =
  "table" | "view" | "collection" | "column" | "savedQuery" | "other"

/**
 * One object the `@` list offers.
 *
 * `label` is what the question will read (`orders`, `orders.status`); the
 * backend never reads it back. `mention` is what it checks: an address, or a
 * saved query's id.
 */
export interface MentionChoice {
  key: string
  kind: MentionKind
  label: string
  /** Where it lives — a schema, a connection — to tell two `orders` apart. */
  detail: string | null
  mention: Mention
}

export const MENTION_KIND_WORD: Record<MentionKind, string> = {
  table: "Table",
  view: "View",
  collection: "Collection",
  column: "Column",
  savedQuery: "Saved query",
  other: "Object",
}

export function mentionIcon(kind: MentionKind) {
  switch (kind) {
    case "view":
      return EyeIcon
    case "collection":
      return Layers01Icon
    case "column":
      return CustomFieldIcon
    case "savedQuery":
      return FileScriptIcon
    case "table":
    case "other":
      return TableIcon
  }
}

/**
 * A chip reads as a word of the line it sits in, and never changes that line.
 *
 * It takes the text's own size, and sits on its baseline: the flex
 * container's baseline is its name's (the only item aligned by baseline — the
 * icon has none and would otherwise lend its bottom edge). With a
 * line-height of 1.25 and a 1px border, the chip is at most 1.25em + 2px tall,
 * below the 20px lines of the field and of the thread, and centred on the
 * text's own centre: inserting or erasing one moves nothing. The border is
 * transparent on every chip so that the dashed one of a missing object keeps
 * the same box. The alignment stories measure all of this.
 */
const CHIP =
  "mx-px inline-flex max-w-full items-center gap-[0.25em] rounded-[0.3em] border border-transparent bg-muted px-[0.3em] py-0 align-baseline leading-[1.25] font-medium text-foreground [&_svg]:size-[1em] [&_svg]:shrink-0 [&_svg]:self-center [&_svg]:text-muted-foreground"

/** Only the name is baseline-aligned: it gives the chip its baseline. */
const NAME = "min-w-0 self-baseline truncate"

/**
 * An object named with `@`: the same chip in the field and in the thread.
 *
 * Its text is `@` and the object's name as the catalog gives it — hostile,
 * and rendered as React text, never as markup. With `onOpen`, it is a button
 * that opens the object; `missing` says the object is gone, and then nothing
 * opens.
 */
export function MentionChip({
  kind,
  label,
  missing = false,
  onOpen,
}: {
  kind: MentionKind
  label: string
  /** The object no longer exists where the chip points. */
  missing?: boolean
  onOpen?: () => void
}) {
  const word = MENTION_KIND_WORD[kind]
  const content = (
    <>
      <HugeiconsIcon
        icon={mentionIcon(kind)}
        strokeWidth={2}
        className="size-[1em]"
        aria-hidden
      />
      <span data-slot="assistant-mention-name" className={NAME}>
        @{label}
      </span>
      {missing ? (
        <span className="shrink-0 self-baseline font-normal text-muted-foreground">
          · not found
        </span>
      ) : null}
    </>
  )

  if (missing)
    return (
      <ContentHint content={`${word} ${label} no longer exists`}>
        <span
          data-slot="assistant-mention"
          data-missing=""
          // Dimmed by colour and outline, not by opacity: the name must stay
          // readable at the contrast the theme guarantees.
          className={cn(
            CHIP,
            "border-dashed border-border bg-transparent text-muted-foreground"
          )}
        >
          {content}
        </span>
      </ContentHint>
    )

  if (onOpen)
    return (
      // Its own menu, `Open object`: a right click on the chip is not one on
      // the question around it.
      <ContextMenu>
        <ContextMenuTrigger
          render={
            <Button
              variant="ghost"
              type="button"
              data-slot="assistant-mention"
              aria-label={`Open ${word.toLowerCase()} ${label}`}
              className={cn(
                // A font size resets an earlier leading in class merging.
                // Apply the chip's inline line height after the inherited size.
                "text-[length:inherit]",
                CHIP,
                "h-auto min-w-0 cursor-pointer hover:bg-accent hover:text-accent-foreground"
              )}
              onClick={onOpen}
            />
          }
        >
          {content}
        </ContextMenuTrigger>
        <ActionMenuContent
          surface="assistantMention"
          sources={{
            assistant: {
              state: { answering: false },
              actions: { openObject: onOpen },
            },
          }}
        />
      </ContextMenu>
    )

  return (
    <ContentHint content={`${word} ${label}`}>
      <span data-slot="assistant-mention" className={CHIP}>
        {content}
      </span>
    </ContentHint>
  )
}
