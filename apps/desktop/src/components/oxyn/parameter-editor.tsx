import { HugeiconsIcon } from "@hugeicons/react"
import { Add01Icon, Delete02Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { MAX_PARAMETERS, PARAMETER_KINDS } from "@/lib/ipc/consoles"
import type { ParameterKind } from "@/lib/ipc/consoles"

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

/**
 * Positional bound values (`$1`, `?`), typed by hand.
 *
 * Values are bound by the driver: never spliced into the SQL, never saved with
 * the query, never written to history. The error shown here comes from the
 * backend and names a position and a type, never a value (I-03).
 */
export function ParameterEditor({
  rows,
  onChange,
  error,
}: {
  rows: Array<ParameterRow>
  onChange: (rows: Array<ParameterRow>) => void
  error?: string | null
}) {
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
      {error ? (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
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
                  aria-label={`Parameter ${position} value`}
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
