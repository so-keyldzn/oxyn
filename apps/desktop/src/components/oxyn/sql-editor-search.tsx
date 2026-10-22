import * as React from "react"
import { createRoot } from "react-dom/client"
import type { Root } from "react-dom/client"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  ArrowUp01Icon,
  Cancel01Icon,
  Search01Icon,
} from "@hugeicons/core-free-icons"
import {
  SearchQuery,
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  replaceAll,
  replaceNext,
  search,
  selectMatches,
  setSearchQuery,
} from "@codemirror/search"
import { runScopeHandlers } from "@codemirror/view"
import type { EditorView, Panel, ViewUpdate } from "@codemirror/view"

import { Button } from "@/components/ui/button"
import { InputGroup, InputGroupAddon } from "@/components/ui/input-group"
import { Kbd } from "@/components/ui/kbd"
import { Toggle } from "@/components/ui/toggle"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { InputGroupTextInput } from "./text-field"

/**
 * The most matches the panel counts. The editor holds one statement or a
 * script, not a file: past this the count says « 1,000+ » rather than walk
 * the text again at every keystroke.
 */
const MATCH_LIMIT = 1000

type Matches =
  | { state: "idle" }
  | { state: "invalid" }
  | { state: "counted"; count: number; capped: boolean }

function matchesOf(view: EditorView, query: SearchQuery): Matches {
  if (query.search === "") return { state: "idle" }
  if (!query.valid) return { state: "invalid" }
  const cursor = query.getCursor(view.state)
  let count = 0
  while (count <= MATCH_LIMIT && !cursor.next().done) count += 1
  return count > MATCH_LIMIT
    ? { state: "counted", count: MATCH_LIMIT, capped: true }
    : { state: "counted", count, capped: false }
}

/** What the counter says, in words. Pure, so it is tested without a DOM. */
export function matchesSummary(matches: Matches): string {
  switch (matches.state) {
    case "idle":
      return ""
    case "invalid":
      return "Invalid regular expression"
    case "counted":
      if (matches.count === 0) return "No results"
      if (matches.capped)
        return `${matches.count.toLocaleString("en-US")}+ results`
      return matches.count === 1
        ? "1 result"
        : `${matches.count.toLocaleString("en-US")} results`
  }
}

/** The query with one change, committed to the editor's search state. */
function commit(
  view: EditorView,
  query: SearchQuery,
  change: Partial<{
    search: string
    replace: string
    caseSensitive: boolean
    regexp: boolean
    wholeWord: boolean
  }>
) {
  const next = new SearchQuery({
    search: query.search,
    replace: query.replace,
    caseSensitive: query.caseSensitive,
    regexp: query.regexp,
    wholeWord: query.wholeWord,
    literal: query.literal,
    ...change,
  })
  if (!next.eq(query)) view.dispatch({ effects: setSearchQuery.of(next) })
}

/** A button of the bar that runs one of CodeMirror's own search commands. */
function BarButton({
  label,
  shortcut,
  onClick,
  disabled,
  children,
}: {
  label: string
  shortcut?: React.ReactNode
  onClick: () => void
  disabled?: boolean
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={label}
            disabled={disabled}
            onClick={onClick}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent>
        {label}
        {shortcut}
      </TooltipContent>
    </Tooltip>
  )
}

/** A search option, pressed or not, written inside the field it refines. */
function Option({
  label,
  pressed,
  onPressedChange,
  children,
}: {
  label: string
  pressed: boolean
  onPressedChange: (pressed: boolean) => void
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Toggle
            size="sm"
            aria-label={label}
            pressed={pressed}
            onPressedChange={onPressedChange}
            className="h-6 min-w-6 rounded-md px-1 font-mono text-xs"
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

/**
 * Find and replace of the SQL editor: CodeMirror's own search — its matches,
 * its commands, its keys — behind the components of the interface instead of
 * the browser's default controls.
 *
 * The query lives in the editor's state and nowhere else: each change is
 * committed there, and the bar is drawn again from it, so that `⌘F` with a
 * selection, `⌘G` and the highlighted matches all agree with what the bar
 * shows.
 */
function EditorSearch({
  view,
  query,
  matches,
  readOnly,
}: {
  view: EditorView
  query: SearchQuery
  matches: Matches
  readOnly: boolean
}) {
  const field = React.useRef<HTMLInputElement>(null)
  // The bar opens on the field, with what it holds selected.
  React.useEffect(() => {
    field.current?.focus()
    field.current?.select()
  }, [])

  const onKeyDown =
    (enter: (event: React.KeyboardEvent) => void) =>
    (event: React.KeyboardEvent) => {
      // The editor's own keys of the panel — Escape, ⌘G — first.
      if (runScopeHandlers(view, event.nativeEvent, "search-panel")) {
        event.preventDefault()
      } else if (event.key === "Enter") {
        event.preventDefault()
        enter(event)
      }
    }

  const summary = matchesSummary(matches)
  const noMatch = matches.state === "counted" && matches.count === 0
  const canStep = matches.state === "counted" && matches.count > 0

  return (
    <div className="flex flex-col gap-1.5 p-2 font-sans text-sm">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <InputGroup className="h-8 w-72 max-w-full min-w-40 shrink">
          <InputGroupAddon>
            <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
          </InputGroupAddon>
          <InputGroupTextInput
            ref={field}
            value={query.search}
            onChange={(event) =>
              commit(view, query, { search: event.target.value })
            }
            onKeyDown={onKeyDown((event) =>
              (event.shiftKey ? findPrevious : findNext)(view)
            )}
            placeholder="Find"
            aria-label="Find"
            aria-invalid={matches.state === "invalid" || undefined}
            name="search"
            // Where CodeMirror puts the focus when ⌘F is pressed again.
            {...{ "main-field": "true" }}
          />
          <InputGroupAddon align="inline-end">
            <Option
              label="Match case"
              pressed={query.caseSensitive}
              onPressedChange={(caseSensitive) =>
                commit(view, query, { caseSensitive })
              }
            >
              Aa
            </Option>
            <Option
              label="Whole word"
              pressed={query.wholeWord}
              onPressedChange={(wholeWord) =>
                commit(view, query, { wholeWord })
              }
            >
              ab
            </Option>
            <Option
              label="Regular expression"
              pressed={query.regexp}
              onPressedChange={(regexp) => commit(view, query, { regexp })}
            >
              .*
            </Option>
          </InputGroupAddon>
        </InputGroup>
        <BarButton
          label="Previous match"
          shortcut={<Kbd>⇧↵</Kbd>}
          disabled={!canStep}
          onClick={() => findPrevious(view)}
        >
          <HugeiconsIcon icon={ArrowUp01Icon} strokeWidth={2} />
        </BarButton>
        <BarButton
          label="Next match"
          shortcut={<Kbd>↵</Kbd>}
          disabled={!canStep}
          onClick={() => findNext(view)}
        >
          <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
        </BarButton>
        <Button
          size="sm"
          variant="ghost"
          disabled={!canStep}
          onClick={() => selectMatches(view)}
        >
          Select all
        </Button>
        <p
          role="status"
          className={cn(
            "min-w-0 truncate text-xs",
            noMatch || matches.state === "invalid"
              ? "text-warning"
              : "text-muted-foreground"
          )}
        >
          {summary}
        </p>
        <BarButton
          label="Close"
          shortcut={<Kbd>Esc</Kbd>}
          onClick={() => closeSearchPanel(view)}
        >
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
        </BarButton>
      </div>
      {readOnly ? null : (
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          <InputGroup className="h-8 w-72 max-w-full min-w-40 shrink">
            <InputGroupTextInput
              value={query.replace}
              onChange={(event) =>
                commit(view, query, { replace: event.target.value })
              }
              onKeyDown={onKeyDown(() => replaceNext(view))}
              placeholder="Replace"
              aria-label="Replace"
              name="replace"
            />
          </InputGroup>
          <Button
            size="sm"
            variant="outline"
            disabled={!canStep}
            onClick={() => replaceNext(view)}
          >
            Replace
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!canStep}
            onClick={() => replaceAll(view)}
          >
            Replace all
          </Button>
        </div>
      )}
    </div>
  )
}

/** The panel CodeMirror mounts under the text, drawn by React. */
class EditorSearchPanel implements Panel {
  readonly dom = document.createElement("div")
  private readonly root: Root
  private query: SearchQuery

  constructor(private readonly view: EditorView) {
    this.dom.setAttribute("role", "search")
    this.dom.setAttribute("aria-label", "Find and replace")
    this.query = getSearchQuery(view.state)
    this.root = createRoot(this.dom)
    this.render()
  }

  private render() {
    this.root.render(
      <EditorSearch
        view={this.view}
        query={this.query}
        matches={matchesOf(this.view, this.query)}
        readOnly={this.view.state.readOnly}
      />
    )
  }

  update(update: ViewUpdate) {
    const query = getSearchQuery(update.state)
    if (
      update.docChanged ||
      update.selectionSet ||
      update.startState.readOnly !== update.state.readOnly ||
      !query.eq(this.query)
    ) {
      this.query = query
      this.render()
    }
  }

  destroy() {
    // Never while React is rendering: the view may be torn down by one of its
    // own effects.
    setTimeout(() => this.root.unmount(), 0)
  }
}

/**
 * The editor's find and replace, as a CodeMirror extension: `⌘F` opens it, and
 * the editor's search keys and match highlighting are the library's.
 */
export const sqlEditorSearch = search({
  createPanel: (view) => new EditorSearchPanel(view),
})
