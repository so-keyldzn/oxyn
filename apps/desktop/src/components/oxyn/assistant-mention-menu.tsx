import * as React from "react"
import { createPortal } from "react-dom"
import { HugeiconsIcon } from "@hugeicons/react"
import { useLexicalComposerContext } from "@lexical/react/LexicalComposerContext"
import {
  LexicalTypeaheadMenuPlugin,
  MenuOption,
} from "@lexical/react/LexicalTypeaheadMenuPlugin"
import type { MenuTextMatch } from "@lexical/react/LexicalTypeaheadMenuPlugin"
import { $createTextNode, COMMAND_PRIORITY_NORMAL } from "lexical"
import type { TextNode } from "lexical"

import { MENTION_KIND_WORD, mentionIcon } from "./assistant-mention-chip"
import type { MentionChoice } from "./assistant-mention-chip"
import { $createMentionNode } from "./assistant-mention-node"
import { placeList } from "./assistant-mention-placement"
import type { ListPlacement } from "./assistant-mention-placement"
import { Item, ItemContent, ItemMedia, ItemTitle } from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

/** What the list has to show for the text typed after `@`. */
export type MentionResults =
  | { status: "idle" }
  | { status: "loading" }
  | {
      status: "ready"
      choices: ReadonlyArray<MentionChoice>
      /** Part of the answer is still coming: what is shown is kept. */
      completing?: boolean
    }
  | { status: "failed"; message: string }

const LIST_NAME = "Objects to mention"
/** The id Lexical points the field's `aria-controls` at. */
const LIST_ID = "typeahead-menu"

/** The longest text after `@` still read as a search, in characters. */
const MAX_QUERY = 64

/** The id Lexical points the field at for the option at `index`. */
const optionId = (index: number) => `typeahead-item-${index}`

/**
 * `@` at the start or after a space, then anything but a space or another
 * `@`. A dot is kept — `orders.st` searches a column —, where Lexical's basic
 * trigger would stop at it.
 */
export function mentionMatch(text: string): MenuTextMatch | null {
  const found = new RegExp(`(^|\\s)(@([^\\s@]{0,${MAX_QUERY}}))$`).exec(text)
  if (!found) return null
  const [, lead = "", replaceable = "", query = ""] = found
  return {
    leadOffset: found.index + lead.length,
    matchingString: query,
    replaceableString: replaceable,
  }
}

class ChoiceOption extends MenuOption {
  readonly choice: MentionChoice
  constructor(choice: MentionChoice) {
    super(choice.key)
    this.choice = choice
  }
}

/**
 * The `@` list: filtered from the keyboard, the caret staying in the field.
 *
 * Lexical owns the keys — arrows move, Enter and Tab choose, Escape closes —
 * and points the field's `aria-activedescendant` at the highlighted option.
 * Its priority is above the composer's: while the list is open, Enter chooses
 * and never sends, Escape closes and never stops a running answer.
 */
export function MentionMenuPlugin({
  results,
  onQuery,
}: {
  results: MentionResults
  onQuery: (query: string | null) => void
}) {
  const [editor] = useLexicalComposerContext()
  const options = React.useMemo(
    () =>
      results.status === "ready"
        ? results.choices.map((choice) => new ChoiceOption(choice))
        : [],
    [results]
  )

  const choose = React.useCallback(
    (option: ChoiceOption, typed: TextNode | null, closeMenu: () => void) => {
      editor.update(() => {
        const chip = $createMentionNode(option.choice)
        if (typed) typed.replace(chip)
        // A space after, so what is typed next is not glued to the chip.
        const space = $createTextNode(" ")
        chip.insertAfter(space)
        space.select(1, 1)
      })
      closeMenu()
    },
    [editor]
  )

  return (
    <LexicalTypeaheadMenuPlugin<ChoiceOption>
      options={options}
      triggerFn={mentionMatch}
      onQueryChange={onQuery}
      onClose={() => onQuery(null)}
      onSelectOption={choose}
      commandPriority={COMMAND_PRIORITY_NORMAL}
      preselectFirstItem={options.length > 0}
      menuRenderFn={(
        anchor,
        { selectedIndex, selectOptionAndCleanUp, setHighlightedIndex },
        query
      ) =>
        anchor.current
          ? createPortal(
              <MentionList
                anchor={anchor.current}
                field={editor.getRootElement()}
                results={results}
                options={options}
                query={query}
                active={selectedIndex}
                onChoose={selectOptionAndCleanUp}
                onActivate={setHighlightedIndex}
              />,
              anchor.current
            )
          : null
      }
    />
  )
}

/** Follows the field's box, and the window's, while the list is open. */
function usePlacement(field: HTMLElement | null) {
  const [placement, setPlacement] = React.useState<ListPlacement | null>(null)
  React.useLayoutEffect(() => {
    // The whole field, hint row included: the list must not cover it.
    const box = field?.closest<HTMLElement>("[data-slot=input-group]") ?? field
    if (!box) return
    const update = () => {
      const rect = box.getBoundingClientRect()
      setPlacement(
        placeList(
          {
            top: rect.top,
            bottom: rect.bottom,
            left: rect.left,
            width: rect.width,
          },
          {
            width: document.documentElement.clientWidth,
            height: document.documentElement.clientHeight,
          }
        )
      )
    }
    update()
    const resized = new ResizeObserver(update)
    resized.observe(box)
    window.addEventListener("resize", update)
    window.addEventListener("scroll", update, true)
    return () => {
      resized.disconnect()
      window.removeEventListener("resize", update)
      window.removeEventListener("scroll", update, true)
    }
  }, [field])
  return placement
}

function MentionList({
  anchor,
  field,
  results,
  options,
  query,
  active,
  onChoose,
  onActivate,
}: {
  anchor: HTMLElement
  field: HTMLElement | null
  results: MentionResults
  options: ReadonlyArray<ChoiceOption>
  query: string
  active: number | null
  onChoose: (option: ChoiceOption) => void
  onActivate: (index: number) => void
}) {
  const placement = usePlacement(field)

  // Lexical makes its anchor the listbox — `role`, a "Typeahead menu" name,
  // the id the field's `aria-controls` names. The listbox is this list
  // instead: it is what scrolls, and a region that scrolls must be focusable
  // (axe `scrollable-region-focusable`), which no child of a listbox may be.
  // Watched rather than undone once: Lexical detaches and re-attaches the
  // anchor at every keystroke, in an effect that runs after this component's.
  React.useEffect(() => {
    const yield_ = () => {
      if (anchor.getAttribute("role") !== "presentation")
        anchor.setAttribute("role", "presentation")
      if (anchor.hasAttribute("aria-label"))
        anchor.removeAttribute("aria-label")
      if (anchor.id === LIST_ID) anchor.removeAttribute("id")
    }
    yield_()
    const watch = new MutationObserver(yield_)
    watch.observe(anchor, { attributeFilter: ["role", "aria-label", "id"] })
    return () => watch.disconnect()
  }, [anchor])

  // One active option, whoever moved it: the field points at the one the
  // list shows active — Lexical moves the pointer for the arrows, not for the
  // mouse —, and at none when the list is empty. A passive effect: it runs
  // after Lexical's layout effect has set it.
  React.useEffect(() => {
    if (!field) return
    if (active === null || options.length === 0)
      field.removeAttribute("aria-activedescendant")
    else field.setAttribute("aria-activedescendant", optionId(active))
  }, [field, active, options])

  // The query this list answers, once answered. Lexical commits it in a
  // transition after the keystroke, then puts the highlight back on the first
  // row in a passive effect: an arrow pressed before that is undone. Set in
  // the same flush of effects, so whoever waits on it — a story on a loaded
  // machine — moves the highlight after the reset, never before.
  const list = React.useRef<HTMLDivElement>(null)
  React.useEffect(() => {
    list.current?.setAttribute("data-query", query)
  }, [query])

  // The active option stays in sight as the arrows move it.
  React.useEffect(() => {
    if (active === null) return
    options[active]?.ref?.current?.scrollIntoView({ block: "nearest" })
  }, [active, options])

  return (
    <div
      data-slot="assistant-mention-list"
      ref={list}
      data-side={placement?.side}
      id={LIST_ID}
      role="listbox"
      aria-label={LIST_NAME}
      // A region that scrolls must be reachable by the keyboard. It is never
      // a tab stop in practice: while the list is open, Tab chooses the
      // active row (Lexical), and a closed list is not in the document. A
      // click on it, scrollbar included, leaves the caret in the field.
      tabIndex={0}
      onMouseDown={(event) => event.preventDefault()}
      style={
        placement
          ? {
              position: "fixed",
              left: placement.left,
              width: placement.width,
              maxHeight: placement.maxHeight,
              top: placement.top ?? undefined,
              bottom: placement.bottom ?? undefined,
            }
          : // Measured before the first paint; hidden until then.
            { position: "fixed", visibility: "hidden" }
      }
      className="z-50 flex flex-col gap-0.5 overflow-y-auto overscroll-contain rounded-lg bg-popover p-1 text-sm text-popover-foreground shadow-md ring-1 ring-foreground/10"
    >
      {options.map((option, index) => {
        const { choice } = option
        const isActive = active === index
        return (
          <Item
            key={option.key}
            ref={(element: HTMLElement | null) => {
              option.setRefElement(element)
            }}
            id={optionId(index)}
            role="option"
            aria-selected={isActive}
            data-active={isActive ? "" : undefined}
            size="xs"
            // The active state is the only highlight: no hover style of its
            // own, so the pointer and the arrows never light two rows.
            className="shrink-0 cursor-default flex-nowrap rounded-md px-2 py-1.5 data-active:bg-accent data-active:text-accent-foreground"
            // The caret stays in the field: a click chooses, it does not
            // take the focus.
            onMouseDown={(event: React.MouseEvent) => event.preventDefault()}
            // On a move, not on enter: a list that scrolls under a still
            // pointer must not take the active row from the arrows.
            onMouseMove={() => {
              if (!isActive) onActivate(index)
            }}
            onClick={() => onChoose(option)}
          >
            <ItemMedia variant="icon">
              <HugeiconsIcon
                icon={mentionIcon(choice.kind)}
                strokeWidth={2}
                aria-hidden
              />
            </ItemMedia>
            <ItemContent className="min-w-0">
              <ItemTitle className="block w-full truncate">
                {choice.label}
              </ItemTitle>
            </ItemContent>
            <span className="shrink-0 text-xs text-muted-foreground">
              {choice.detail ?? MENTION_KIND_WORD[choice.kind]}
            </span>
          </Item>
        )
      })}
      <MentionStatus results={results} query={query} />
    </div>
  )
}

/**
 * Whatever is not a choice: the wait, the empty answer, the failure.
 *
 * An option that cannot be chosen, so the list is never an empty listbox —
 * which a screen reader announces as nothing at all. Lexical does not know
 * it: the arrows never land on it.
 */
function MentionStatus({
  results,
  query,
}: {
  results: MentionResults
  query: string
}) {
  const line = "shrink-0 px-2 py-1.5 text-xs text-muted-foreground"
  const status = (text: React.ReactNode, className?: string) => (
    <div
      role="option"
      aria-disabled="true"
      aria-selected={false}
      className={cn(line, className)}
    >
      {text}
    </div>
  )
  // The wait is a row the size of an answer, never an empty verdict: "No
  // matching object" is said only once an answer said so.
  const waiting = status(
    <>
      <span>Loading…</span>
      <Skeleton aria-hidden className="h-3 w-3/4" />
      <Skeleton aria-hidden className="h-3 w-1/2" />
    </>,
    "flex flex-col gap-1.5"
  )
  switch (results.status) {
    case "idle":
      return query === ""
        ? status("Type to search tables, views, columns and queries")
        : null
    case "loading":
      return waiting
    case "failed":
      return status(results.message, "text-destructive")
    case "ready":
      if (results.choices.length > 0) return null
      if (results.completing) return waiting
      return query === ""
        ? status("Type to search tables, views, columns and queries")
        : status("No matching object")
  }
}
