import * as React from "react"
import CodeMirror from "@uiw/react-codemirror"
import { PostgreSQL, SQLite, StandardSQL, sql } from "@codemirror/lang-sql"
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language"
import { Prec } from "@codemirror/state"
import { EditorView, keymap } from "@codemirror/view"
import { tags } from "@lezer/highlight"
import { TEXT_FIELD_DOM_ATTRIBUTES } from "./text-field"

function dialectFor(driver: string) {
  switch (driver) {
    case "postgres":
      return PostgreSQL
    case "sqlite":
      return SQLite
    default:
      return StandardSQL
  }
}

// Colours come from the CSS tokens, so the editor follows the theme switch
// without rebuilding its extensions.
const theme = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "var(--background)",
    color: "var(--foreground)",
    // The reading preset, not a fixed size: Comfortable must move the SQL as
    // it moves the grid (docs/UX-SPEC.md, « Lisibilité et hauteur de grille »).
    fontSize: "var(--reading-text)",
  },
  ".cm-content": {
    fontFamily: "var(--font-mono)",
    caretColor: "var(--primary)",
    padding: "8px 0",
  },
  ".cm-gutters": {
    backgroundColor: "var(--card)",
    color: "var(--muted-foreground)",
    borderRight: "1px solid var(--border)",
  },
  ".cm-activeLine": {
    backgroundColor: "color-mix(in oklab, var(--accent) 60%, transparent)",
  },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection":
    {
      backgroundColor:
        "color-mix(in oklab, var(--primary) 30%, transparent) !important",
    },
  ".cm-cursor": { borderLeftColor: "var(--primary)" },
  "&.cm-focused": { outline: "none" },
  ".cm-tooltip": {
    backgroundColor: "var(--popover)",
    color: "var(--popover-foreground)",
    border: "1px solid var(--border)",
    borderRadius: "6px",
  },
})

const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: tags.keyword, color: "var(--primary-text)", fontWeight: "500" },
    { tag: [tags.string, tags.special(tags.string)], color: "var(--success)" },
    { tag: [tags.number, tags.bool, tags.null], color: "var(--warning)" },
    {
      tag: tags.comment,
      color: "var(--muted-foreground)",
      fontStyle: "italic",
    },
    { tag: [tags.typeName, tags.standard(tags.name)], color: "var(--chart-4)" },
    { tag: tags.operator, color: "var(--muted-foreground)" },
  ])
)

/** What ⌘↵ targets: the selection when there is one, else the cursor. */
export type EditorTarget =
  | { kind: "selection"; start: number; end: number }
  | { kind: "statement"; cursor: number }

/** Offsets are UTF-16 code units, CodeMirror's own, which the backend converts. */
export function targetOf(view: EditorView): EditorTarget {
  const range = view.state.selection.main
  return range.empty
    ? { kind: "statement", cursor: range.head }
    : { kind: "selection", start: range.from, end: range.to }
}

/**
 * The SQL console editor.
 *
 * `⌘↵` runs what the selection or cursor targets, `⌘⇧↵` runs everything, `⌘S`
 * saves, `Esc` cancels while running. The text is sent as written — it is the
 * user's SQL, not a statement Oxyn composes (I-10).
 */
export function SqlEditor({
  value,
  onChange,
  driver,
  running = false,
  readOnly = false,
  onRun,
  onRunAll,
  onSave,
  onCancel,
  onTargetChange,
  onBlur,
  onEditor,
  autoFocus = false,
}: {
  value: string
  onChange: (value: string) => void
  driver: string
  running?: boolean
  readOnly?: boolean
  onRun: (target: EditorTarget) => void
  onRunAll?: () => void
  onSave?: () => void
  onCancel?: () => void
  onTargetChange?: (kind: EditorTarget["kind"]) => void
  /** Focus left the editor: a pending draft is written now (ADR-0024). */
  onBlur?: () => void
  /** The CodeMirror view, to focus it or put text in it. */
  onEditor?: (view: EditorView) => void
  autoFocus?: boolean
}) {
  // Handlers change every render; the keymap is built once and reads the
  // latest through a ref, so CodeMirror does not reconfigure on each keystroke.
  const handlers = React.useRef({
    onRun,
    onRunAll,
    onSave,
    onCancel,
    onTargetChange,
    onBlur,
    running,
    readOnly,
  })
  handlers.current = {
    onRun,
    onRunAll,
    onSave,
    onCancel,
    onTargetChange,
    onBlur,
    running,
    readOnly,
  }

  const extensions = React.useMemo(
    () => [
      sql({ dialect: dialectFor(driver), upperCaseKeywords: true }),
      theme,
      highlight,
      EditorView.lineWrapping,
      // The editable surface itself carries the name: a label on the wrapper
      // does not reach the element screen readers focus.
      //
      // `tabindex="0"` adds no tab stop — a `contenteditable` host is already
      // one — but it says so: axe only counts native controls and explicit
      // tabindexes as focusable, and flags the editor as a scroll region the
      // keyboard cannot reach as soon as a statement is taller than the panel.
      EditorView.contentAttributes.of({
        "aria-label": "SQL editor",
        tabindex: "0",
        // No correction nor typographic quotes in SQL (ADR-0041 § 8).
        ...TEXT_FIELD_DOM_ATTRIBUTES,
      }),
      EditorView.updateListener.of((update) => {
        if (update.selectionSet)
          handlers.current.onTargetChange?.(
            update.state.selection.main.empty ? "statement" : "selection"
          )
      }),
      EditorView.domEventHandlers({
        blur: () => {
          handlers.current.onBlur?.()
          return false
        },
      }),
      Prec.highest(
        keymap.of([
          // A read-only editor offers selection and copy, and no shortcut that
          // runs or writes (docs/UX-SPEC.md, « Consultation locale des
          // requêtes »): ⌘↵ on a library view must submit nothing.
          {
            key: "Mod-Enter",
            preventDefault: true,
            run: (view) => {
              if (handlers.current.readOnly) return true
              if (!handlers.current.running)
                handlers.current.onRun(targetOf(view))
              return true
            },
          },
          {
            key: "Mod-Shift-Enter",
            preventDefault: true,
            run: () => {
              if (handlers.current.readOnly) return true
              if (!handlers.current.running) handlers.current.onRunAll?.()
              return true
            },
          },
          {
            key: "Mod-s",
            preventDefault: true,
            run: () => {
              if (handlers.current.readOnly) return true
              handlers.current.onSave?.()
              return true
            },
          },
          {
            key: "Escape",
            run: () => {
              if (!handlers.current.running || !handlers.current.onCancel)
                return false
              handlers.current.onCancel()
              return true
            },
          },
        ])
      ),
    ],
    [driver]
  )

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      extensions={extensions}
      theme="none"
      height="100%"
      autoFocus={autoFocus}
      readOnly={readOnly}
      onCreateEditor={onEditor}
      className="h-full min-h-0 overflow-hidden"
      basicSetup={{
        lineNumbers: true,
        highlightActiveLine: true,
        foldGutter: false,
        autocompletion: true,
        bracketMatching: true,
        closeBrackets: true,
      }}
    />
  )
}
