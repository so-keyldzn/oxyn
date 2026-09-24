import * as React from "react"

import { HugeiconsIcon } from "@hugeicons/react"
import { Add01Icon, Delete02Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { MAX_PARAMETERS, PARAMETER_KINDS } from "@/lib/ipc/consoles"
import type { ParameterKind, ParameterRefusal } from "@/lib/ipc/consoles"

/** One positional value. `id` survives the removal of a neighbour. */
export interface ParameterRow {
  id: string
  type: ParameterKind
  text: string
}

export function newParameterRow(): ParameterRow {
  // A new row binds NULL until a type is chosen: an untyped value never
  // reaches the server as a guessed type.
  return { id: crypto.randomUUID(), type: "null", text: "" }
}

// The clipboard, drag-and-drop and the native context menu (Copy, Share,
// Services) are channels by which a bound value would leave the window
// (I-03, UX-SPEC § Valeurs liées d'une console). Typing, selecting and
// pasting from the keyboard stay possible: only extraction is refused.
//
// The value field carries `data-select="none"` locally, which an unlayered
// rule in styles.css turns into `user-select: none` — unlayered so it beats
// the global `user-select: text` rule of `@layer base` outright, without
// touching that rule for every other input. Its effect on the field's own
// value is engine-dependent though: Chromium/WebView2 ignores `user-select`
// for form field content, so this is defense in depth, not the barrier. The
// barrier is the four blocked events below: copy, cut, drag and the context
// menu are what actually closes extraction, on every engine. The residual
// risk of a selection read by WebKit's Look Up, Services, or accessibility
// stays OPEN — it falls under the multi-engine verification noted "non
// évaluée" in the risk table of docs/ARCHITECTURE.md (row "Trois moteurs
// web") and "non faite" in docs/IMPLEMENTATION-PLAN.md § Migration vers
// l'interface Tauri.
const refuseExtraction = (event: React.SyntheticEvent) => event.preventDefault()

/**
 * Positional bound values (`$1`, `?`), typed by hand.
 *
 * Values are bound by the driver: never spliced into the SQL, never saved with
 * the query, never written to history. `refusal` comes from the backend and
 * names a position and a type, never a value (I-03). When it names a row, that
 * row is marked `aria-invalid`, scrolled into view and focused; a refusal of
 * the parameter *count* (`position: null`) marks no row — there is no single
 * faulty one — and only the alert shows.
 */
export function ParameterEditor({
  rows,
  onChange,
  refusal,
}: {
  rows: Array<ParameterRow>
  onChange: (rows: Array<ParameterRow>) => void
  refusal?: ParameterRefusal | null
}) {
  const alertId = React.useId()
  const faultyPosition = refusal?.position ?? null
  const faultyRef = React.useRef<HTMLInputElement>(null)

  // Brings the faulty row into view and hands it the focus: the panel may
  // already be open (a re-run refused again) or opening now (the panel is
  // shown as a side effect of the refusal, one render earlier). Typing in any
  // row clears `refusal` from console-panel.tsx before this can fire again.
  React.useEffect(() => {
    const field = faultyRef.current
    if (!field) return
    field.scrollIntoView({ block: "nearest" })
    field.focus({ preventScroll: true })
  }, [refusal])

  const update = (id: string, patch: Partial<ParameterRow>) =>
    onChange(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)))

  return (
    <section
      aria-label="Bound parameters"
      className="flex max-h-[300px] flex-col gap-2 overflow-auto border-b bg-card px-3 py-2"
    >
      <p className="text-xs text-muted-foreground">
        Values are bound by the driver. They are never inserted into the SQL
        text, saved with the query, or written to the history.
      </p>
      {refusal ? (
        <Alert id={alertId} variant="destructive">
          <AlertDescription>{refusal.message}</AlertDescription>
        </Alert>
      ) : null}
      {rows.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          No parameter. Add one per placeholder, in order.
        </p>
      ) : (
        <ol className="flex flex-col gap-1.5">
          {rows.map((row, index) => {
            const position = index + 1
            const faulty = position === faultyPosition
            return (
              // Wraps instead of pushing the row past the panel: at 420 px the
              // type, the value and the remove button all stay reachable.
              <li key={row.id} className="flex flex-wrap items-center gap-2">
                <span className="w-6 text-right font-mono text-xs text-muted-foreground tabular-nums">
                  {position}
                </span>
                <NativeSelect
                  aria-label={`Parameter ${position} type`}
                  value={row.type}
                  onChange={(event) =>
                    update(row.id, {
                      type: event.target.value as ParameterKind,
                    })
                  }
                  className="w-44 shrink-0"
                >
                  {PARAMETER_KINDS.map((kind) => (
                    <NativeSelectOption key={kind.kind} value={kind.kind}>
                      {kind.label}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
                <Input
                  ref={faulty ? faultyRef : undefined}
                  aria-label={`Parameter ${position} value`}
                  aria-invalid={faulty}
                  aria-describedby={faulty ? alertId : undefined}
                  value={row.text}
                  // NULL binds no text: the field stays read-only until a type
                  // is chosen.
                  readOnly={row.type === "null"}
                  placeholder={row.type === "null" ? "NULL" : undefined}
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(event) =>
                    update(row.id, { text: event.target.value })
                  }
                  onCopy={refuseExtraction}
                  onCut={refuseExtraction}
                  onDragStart={refuseExtraction}
                  onContextMenu={refuseExtraction}
                  data-select="none"
                  className="h-8 w-64 min-w-40 flex-1 font-mono"
                />
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`Remove parameter ${position}`}
                  onClick={() =>
                    onChange(rows.filter((item) => item.id !== row.id))
                  }
                >
                  <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                </Button>
              </li>
            )
          })}
        </ol>
      )}
      <div>
        <Button
          size="sm"
          variant="outline"
          disabled={rows.length >= MAX_PARAMETERS}
          onClick={() => onChange([...rows, newParameterRow()])}
        >
          <HugeiconsIcon
            icon={Add01Icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Add parameter
        </Button>
      </div>
    </section>
  )
}
