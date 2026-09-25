import * as React from "react"

import { HEADER_HEIGHT, ROW_HEIGHT } from "@/components/oxyn/result-grid"
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable"

/**
 * Lines each side of the splitter keeps, whatever the window and the density.
 * Three: the line being read or typed plus one neighbour on each side. Under
 * that, every arrow key in the grid and every line break in the editor
 * scrolls, and the context the user was reading is gone.
 */
export const MIN_VISIBLE_LINES = 3

/** `ResultFooter` is `min-h-8`, border included. */
const RESULT_FOOTER_HEIGHT = 32
/** CodeMirror's base theme sets `.cm-scroller { line-height: 1.4 }`. */
const EDITOR_LINE_HEIGHT = 1.4
/** `sql-editor.tsx` pads `.cm-content` with 8 px above and below. */
const EDITOR_PADDING = 16
/** Compact reading text, used until the page has been read. */
const READING_TEXT = 13

/**
 * The floors of the two panels, in pixels, for the density on screen.
 *
 * Percentages cannot express them: 15 % of the group is 73 px in a 600 px
 * window, which the grid's header and the panel's footer fill on their own —
 * the footer then says « 250,000 rows shown » over a grid that draws none.
 * The row height and the text size come from the reading density
 * (`--grid-row-height`, `--reading-text`) and are followed when it changes: a
 * floor computed for Compact would crush the grid again in Comfortable.
 */
function useConsoleFloors() {
  const [metrics, setMetrics] = React.useState({
    rowHeight: ROW_HEIGHT,
    readingText: READING_TEXT,
  })
  React.useEffect(() => {
    const read = () => {
      const style = getComputedStyle(document.documentElement)
      const row = Number.parseFloat(style.getPropertyValue("--grid-row-height"))
      const text = Number.parseFloat(style.getPropertyValue("--reading-text"))
      setMetrics((current) => {
        const next = {
          rowHeight: Number.isFinite(row) && row > 0 ? row : ROW_HEIGHT,
          readingText: Number.isFinite(text) && text > 0 ? text : READING_TEXT,
        }
        return current.rowHeight === next.rowHeight &&
          current.readingText === next.readingText
          ? current
          : next
      })
    }
    read()
    const observer = new MutationObserver(read)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-density", "class", "style"],
    })
    return () => observer.disconnect()
  }, [])
  return {
    results:
      HEADER_HEIGHT +
      RESULT_FOOTER_HEIGHT +
      MIN_VISIBLE_LINES * metrics.rowHeight,
    editor: Math.ceil(
      EDITOR_PADDING +
        MIN_VISIBLE_LINES * metrics.readingText * EDITOR_LINE_HEIGHT
    ),
  }
}

/**
 * The layout of one console: toolbar, notice line, bound values, editor over
 * results. Export and the run status live in the result footer, under the
 * rows they describe (docs/UX-SPEC.md, « Lisibilité et hauteur de grille »).
 * Presentational: every part arrives by props, so the whole console is
 * described in stories without a backend.
 *
 * The splitter stops before either side becomes unreadable: each panel keeps
 * `MIN_VISIBLE_LINES` lines of its own content, in pixels.
 */
export function ConsoleView({
  toolbar,
  offline,
  notice,
  draftNotice,
  parameters,
  editor,
  results,
}: {
  toolbar: React.ReactNode
  /** Shown while the console has no session: why it cannot run, and how to. */
  offline?: React.ReactNode
  /** Why this console is here or what just happened. Never a bound value. */
  notice: string | null
  /** Where the recovery draft stands. */
  draftNotice: string
  /** The parameter editor, when open. */
  parameters?: React.ReactNode
  editor: React.ReactNode
  results: React.ReactNode
}) {
  const floors = useConsoleFloors()
  return (
    <div data-slot="console" className="flex h-full min-h-0 flex-col">
      {toolbar}
      {offline}
      {/* Wraps rather than hides: « unsaved » is the one word a narrow window
          must not drop (docs/UX-SPEC.md, « Autosauvegarde des brouillons »). */}
      <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-x-3 border-b px-3 py-1 text-xs text-muted-foreground">
        <span
          role="status"
          dir="auto"
          title={notice ?? undefined}
          className="min-w-0 flex-1 truncate"
        >
          {notice}
        </span>
        <span className="shrink-0">{draftNotice}</span>
      </div>
      {parameters}
      <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
        {/* A number is pixels in react-resizable-panels 4; a unit-less
            string would be a percentage. */}
        <ResizablePanel defaultSize="40" minSize={floors.editor}>
          {editor}
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel defaultSize="60" minSize={floors.results}>
          {results}
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}
