import { HugeiconsIcon } from "@hugeicons/react"
import { Copy01Icon, SourceCodeIcon } from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { DefinitionView } from "@/lib/ipc/metadata"

export function definitionSourceLabel(source: DefinitionView["source"]) {
  switch (source) {
    case "stored":
      return "Stored definition"
    case "reconstructed":
      return "Reconstructed definition"
    default:
      return "Definition of unreported provenance"
  }
}

/**
 * Said by the console that receives a definition kept after a failed refresh
 * or an invalidation: the copy must not look fresher than the panel did.
 */
export const STALE_DEFINITION_NOTICE =
  "This DDL may be outdated: its last refresh failed or a schema change invalidated it. Nothing was executed."

/** The header actions of the DDL tab, shown once a definition is loaded. */
export function DefinitionActions({
  definition,
  stale,
  onCopy,
  onOpenInConsole,
}: {
  definition: DefinitionView
  /** The text shown may be outdated: the console says so too. */
  stale: boolean
  onCopy: (sql: string) => void
  /** Absent when no console can be opened from here. */
  onOpenInConsole?: (sql: string, notice?: string) => void
}) {
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        onClick={() => onCopy(definition.sql)}
      >
        <HugeiconsIcon
          icon={Copy01Icon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        Copy DDL
      </Button>
      {onOpenInConsole ? (
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            onOpenInConsole(
              definition.sql,
              stale ? STALE_DEFINITION_NOTICE : undefined
            )
          }
        >
          <HugeiconsIcon
            icon={SourceCodeIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Open DDL in console
        </Button>
      ) : null}
    </>
  )
}

/**
 * Creation statements, read-only (ADR-0018).
 *
 * The text is the engine's or a reconstruction, and says which; its notes
 * state what it does not cover. It is text in a `pre`: nothing here can run
 * it, and opening it in a console runs nothing either.
 */
export function RelationDefinition({
  definition,
  stale,
}: {
  definition: DefinitionView
  /** The last refresh failed or a DDL invalidated it: said again here. */
  stale: boolean
}) {
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Badge variant="secondary">
          {definitionSourceLabel(definition.source)}
        </Badge>
        <Badge variant="outline">Read only</Badge>
        {stale ? (
          <span className="text-warning">This definition may be outdated.</span>
        ) : null}
      </div>
      {definition.notes.length > 0 ? (
        <ul className="flex max-h-24 flex-col gap-0.5 overflow-auto text-xs text-muted-foreground">
          {definition.notes.map((note, index) => (
            <li key={index}>{note}</li>
          ))}
        </ul>
      ) : null}
      <pre
        data-selectable
        aria-label="Object definition"
        tabIndex={0}
        className="overflow-auto rounded-md border bg-card p-3 font-mono text-xs whitespace-pre outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {definition.sql}
      </pre>
    </div>
  )
}
