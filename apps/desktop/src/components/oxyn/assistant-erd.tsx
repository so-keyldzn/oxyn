import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, SourceCodeIcon } from "@hugeicons/core-free-icons"

import { ERD_MAX_NAMES } from "@/components/oxyn/assistant-markdown-model"
import type { ErdLink, ErdTable } from "@/components/oxyn/erd-model"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import type { CatalogAddress } from "@/lib/ipc/types"

// Loaded with the first diagram shown: xyflow and dagre stay out of the bundle
// every launch parses (docs/PERFORMANCE.md, cold start).
const ErdDiagram = React.lazy(() =>
  import("@/components/oxyn/erd-diagram").then((module) => ({
    default: module.ErdDiagram,
  }))
)

/** A name the answer wrote that matches more than one table. */
export interface ErdAmbiguity {
  written: string
  candidates: Array<string>
}

export type ErdState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | {
      status: "ready"
      tables: Array<ErdTable>
      links: Array<ErdLink>
      /** Related tables the bound left out. */
      omitted: number
      /** Names the catalog does not hold, as the answer wrote them. */
      notFound: Array<string>
      ambiguous: Array<ErdAmbiguity>
      /** Names past `ERD_MAX_NAMES`, not even looked up. */
      ignoredNames: number
    }

function Unresolved({
  state,
}: {
  state: Extract<ErdState, { status: "ready" }>
}) {
  if (
    state.notFound.length === 0 &&
    state.ambiguous.length === 0 &&
    state.ignoredNames === 0
  )
    return null
  return (
    <div className="flex flex-col gap-1 border-t px-3 py-2 text-xs text-muted-foreground">
      {state.notFound.length > 0 ? (
        <p>
          Not found among the objects Oxyn has read:{" "}
          {state.notFound.map((name, index) => (
            <React.Fragment key={name}>
              {index > 0 ? ", " : null}
              <code className="font-mono text-foreground">{name}</code>
            </React.Fragment>
          ))}
          . Nothing is drawn for them.
        </p>
      ) : null}
      {state.ambiguous.map((entry) => (
        <p key={entry.written}>
          <code className="font-mono text-foreground">{entry.written}</code>{" "}
          matches {entry.candidates.join(", ")}: not drawn until qualified.
        </p>
      ))}
      {state.ignoredNames > 0 ? (
        <p>
          {state.ignoredNames} more{" "}
          {state.ignoredNames === 1 ? "name" : "names"} not looked up: a block
          is read up to {ERD_MAX_NAMES}.
        </p>
      ) : null}
    </div>
  )
}

function Body({
  state,
  onOpenObject,
  onCopyName,
  onRetry,
}: {
  state: ErdState
  onOpenObject?: (address: CatalogAddress) => void
  onCopyName?: (table: ErdTable) => void
  onRetry?: () => void
}) {
  switch (state.status) {
    case "loading":
      return (
        <p
          aria-busy
          className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground"
        >
          <Spinner aria-hidden />
          Reading the tables and their keys from the catalog…
        </p>
      )
    case "error":
      return (
        <div className="p-2">
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>The diagram could not be read.</AlertTitle>
            <AlertDescription>
              <p dir="auto" className="font-mono break-words">
                {state.message}
              </p>
              {onRetry ? (
                <Button
                  size="sm"
                  variant="outline"
                  className="mt-2"
                  onClick={onRetry}
                >
                  Try again
                </Button>
              ) : null}
            </AlertDescription>
          </Alert>
        </div>
      )
    case "ready":
      return (
        <>
          {state.tables.length > 0 ? (
            <React.Suspense
              fallback={
                <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
                  <Spinner />
                  Drawing the diagram…
                </div>
              }
            >
              <ErdDiagram
                tables={state.tables}
                links={state.links}
                omitted={state.omitted}
                onOpenObject={onOpenObject}
                onCopyName={onCopyName}
              />
            </React.Suspense>
          ) : (
            <p className="px-3 py-2 text-xs text-muted-foreground">
              No table to draw.
            </p>
          )}
          <Unresolved state={state} />
        </>
      )
  }
}

/**
 * An `erd` block of an answer, drawn as a diagram of the tables it names.
 *
 * The block is a list of names, and names are requests: what is drawn is what
 * the catalog holds under them, read by the backend. The source stays one
 * click away, since it is what the model actually wrote.
 */
export function AssistantErd({
  source,
  state,
  onOpenObject,
  onCopyName,
  onRetry,
}: {
  /** The block's text, as the model wrote it. */
  source: string
  state: ErdState
  onOpenObject?: (address: CatalogAddress) => void
  /** `Copy name` of a table's context menu. */
  onCopyName?: (table: ErdTable) => void
  onRetry?: () => void
}) {
  const [showSource, setShowSource] = React.useState(false)
  const sourceId = React.useId()
  return (
    <figure
      data-slot="assistant-erd"
      className="flex min-w-0 flex-col overflow-hidden rounded-lg border bg-card"
    >
      <figcaption className="flex h-8 items-center justify-between gap-2 border-b px-2.5 text-xs text-muted-foreground">
        <span>Table diagram</span>
        <Button
          size="xs"
          variant="ghost"
          aria-expanded={showSource}
          aria-controls={sourceId}
          onClick={() => setShowSource((shown) => !shown)}
        >
          <HugeiconsIcon
            icon={SourceCodeIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          {showSource ? "Hide source" : "Show source"}
        </Button>
      </figcaption>
      {showSource ? (
        <pre
          id={sourceId}
          data-selectable
          tabIndex={0}
          aria-label="Diagram source"
          className="overflow-x-auto border-b p-3 font-mono text-xs leading-5 whitespace-pre outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <code>{source}</code>
        </pre>
      ) : null}
      <Body
        state={state}
        onOpenObject={onOpenObject}
        onCopyName={onCopyName}
        onRetry={onRetry}
      />
    </figure>
  )
}
