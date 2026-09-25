import * as React from "react"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldLabel,
} from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import type {
  ObjectOperationReview,
  OperationKind,
} from "@/lib/ipc/object-operations"
import type { IncomingKeyRow } from "@/lib/ipc/metadata"
import type { CatalogAddress } from "@/lib/ipc/types"
import { TextInput } from "./text-field"

/** Where the submission stands, as the box shows it. */
export type ReviewPhase =
  | { kind: "idle" }
  | { kind: "running" }
  /** The gate or the host's dialog refused: nothing ran. */
  | { kind: "denied"; reason: string }
  /** The server refused, in its own words. */
  | { kind: "failed"; message: string }
  /** Sent, outcome unknown: never retried (I-13). */
  | { kind: "ambiguous"; message: string }
  /** Nothing was sent: may be submitted again. */
  | { kind: "notSent"; message: string }

/** What the box knows of the foreign keys that reference the table. */
export type DependentsRead =
  { kind: "reading" } | { kind: "failed"; message: string } | { kind: "read" }

export const AMBIGUOUS =
  "The server may have applied this. Nothing will be retried. Refresh the catalog to see the current state."

const ACTION: Record<OperationKind, string> = {
  drop: "Drop",
  truncate: "Truncate",
  rename: "Rename",
}

function objectLabel(kind: string) {
  return kind.replaceAll("_", " ")
}

function sourceLabel(address: CatalogAddress) {
  return [address.catalog, address.namespace, address.relation]
    .filter((segment): segment is string => segment !== null)
    .join(".")
}

/**
 * The review of `Drop…`, `Truncate…` or `Rename…` from the catalog
 * (docs/adr/0042-revue-sur-place-des-operations-destructrices.md).
 *
 * The statement is the backend's, shown whole: what is shown is what is sent.
 * Built not to be confirmed by reflex (docs/UX-SPEC.md, « Les opérations
 * destructrices »): `Cancel` takes the initial focus, Enter alone does
 * nothing, the default button is never the action. On production, or once
 * `CASCADE` is ticked, the action waits for the object's name to be typed —
 * a guard against the wrong row of the tree, not a guarantee: on production
 * the host's own dialog decides after the gate.
 *
 * The box approves nothing. When the gate holds the statement, the caller
 * hands over to `ApprovalDialog`.
 */
export function ObjectOperationReviewDialog({
  open,
  operation,
  column,
  review,
  problem,
  newName,
  onNewNameChange,
  cascade,
  onCascadeChange,
  dependents,
  phase,
  onRun,
  onStop,
  onCancel,
  onRefreshCatalog,
  onOpenInConsole,
}: {
  open: boolean
  operation: OperationKind
  /** Set when a column is renamed. */
  column?: string | null
  /** `null` while the backend composes. */
  review: ObjectOperationReview | null
  /** Why no statement is composed — a name to fix, an object gone. */
  problem: string | null
  newName?: string
  onNewNameChange?: (name: string) => void
  cascade: boolean
  onCascadeChange: (cascade: boolean) => void
  dependents: DependentsRead
  phase: ReviewPhase
  onRun: () => void
  onStop: () => void
  onCancel: () => void
  onRefreshCatalog: () => void
  onOpenInConsole: () => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const typedId = React.useId()
  const renameId = React.useId()
  const cascadeId = React.useId()
  const [typed, setTyped] = React.useState("")
  React.useEffect(() => {
    if (open) setTyped("")
  }, [open])

  const running = phase.kind === "running"
  const settled = phase.kind === "ambiguous" || phase.kind === "failed"
  const destroys = operation !== "rename"
  const production = review?.environment === "production"
  const mustType = review !== null && (production || cascade)
  const typedMatches = !mustType || typed === review.objectName
  const readingDependents =
    review?.dependents.type === "incomingKeys" && dependents.kind === "reading"
  const canRun =
    review !== null &&
    problem === null &&
    !readingDependents &&
    typedMatches &&
    !running &&
    !settled &&
    phase.kind !== "denied"
  const kind = review ? objectLabel(review.relationKind) : "object"
  const target = column ? "column" : kind
  const label = `${ACTION[operation]} ${target}`

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !running) onCancel()
      }}
    >
      <AlertDialogContent
        className="grid-cols-1 data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-xl"
        initialFocus={cancelRef}
        // Enter in a field or on the statement never reaches the action.
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !(event.target instanceof HTMLButtonElement)
          ) {
            event.preventDefault()
          }
        }}
      >
        <AlertDialogHeader>
          <div className="flex flex-wrap items-center gap-2">
            <AlertDialogTitle>{label}</AlertDialogTitle>
            {review ? (
              <EnvironmentBadge environment={review.environment} />
            ) : null}
          </div>
          <AlertDialogDescription className="wrap-anywhere">
            {review ? (
              <>
                This statement will run on{" "}
                <strong className="font-medium text-foreground">
                  <bdi>{review.connectionName}</bdi>
                </strong>
                , on a session of its own.
              </>
            ) : (
              "Composing the statement…"
            )}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {operation === "rename" ? (
          <Field data-invalid={problem !== null || undefined}>
            <FieldLabel htmlFor={renameId}>New name</FieldLabel>
            <TextInput
              id={renameId}
              value={newName ?? ""}
              onChange={(event) => onNewNameChange?.(event.target.value)}
              disabled={running || settled}
              aria-invalid={problem !== null || undefined}
            />
            {problem ? <FieldError>{problem}</FieldError> : null}
          </Field>
        ) : problem ? (
          <Alert variant="destructive">
            <AlertTitle>Nothing to run</AlertTitle>
            <AlertDescription className="wrap-anywhere">
              {problem}
            </AlertDescription>
          </Alert>
        ) : null}

        {review && problem === null ? (
          <pre
            data-selectable
            tabIndex={0}
            aria-label="Statement to run"
            className="max-h-40 overflow-auto rounded-md border bg-card p-3 font-mono text-xs leading-5 break-words whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {review.sql}
          </pre>
        ) : null}

        {review ? (
          <div className="flex flex-col gap-1.5 text-xs text-muted-foreground">
            {destroys ? (
              <DependentsList review={review} dependents={dependents} />
            ) : null}
            {destroys ? (
              <p>
                {review.restrictDependents
                  ? "Without CASCADE, the server refuses if other objects depend on it."
                  : "This database drops the object even if other objects still use it."}
              </p>
            ) : null}
            <p>
              {review.transactionalDdl
                ? "Applied whole or not at all. Once it succeeds it is committed: there is no undo."
                : "This database does not run DDL in a transaction: it cannot be rolled back, and a failure may leave part of it applied."}
            </p>
          </div>
        ) : null}

        {review?.restrictDependents && destroys ? (
          <Field orientation="horizontal">
            <Checkbox
              id={cascadeId}
              checked={cascade}
              disabled={running || settled}
              onCheckedChange={onCascadeChange}
            />
            <FieldLabel htmlFor={cascadeId} className="font-normal">
              CASCADE — also remove what depends on it, which is not all listed
              here
            </FieldLabel>
          </Field>
        ) : null}

        {mustType ? (
          <Field>
            <FieldLabel htmlFor={typedId}>
              Type the {target} name to confirm
            </FieldLabel>
            <TextInput
              id={typedId}
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
              disabled={running || settled}
            />
            <FieldDescription>
              Exactly{" "}
              <bdi className="font-mono break-all">{review.objectName}</bdi>.
            </FieldDescription>
          </Field>
        ) : null}

        <PhaseNotice phase={phase} />

        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={cancelRef} disabled={running}>
            {settled ? "Close" : "Cancel"}
          </AlertDialogCancel>
          {phase.kind === "ambiguous" ? (
            <Button variant="outline" onClick={onRefreshCatalog}>
              Refresh catalog
            </Button>
          ) : phase.kind === "failed" ? (
            <>
              <Button variant="outline" onClick={onRefreshCatalog}>
                Refresh catalog
              </Button>
              {operation === "drop" ? (
                <Button variant="outline" onClick={onOpenInConsole}>
                  Open in console
                </Button>
              ) : null}
            </>
          ) : running ? (
            <Button variant="outline" onClick={onStop}>
              <Spinner data-icon="inline-start" />
              Stop
            </Button>
          ) : (
            <Button
              variant={destroys || production ? "destructive" : "default"}
              disabled={!canRun}
              onClick={onRun}
            >
              {label}
            </Button>
          )}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function DependentsList({
  review,
  dependents,
}: {
  review: ObjectOperationReview
  dependents: DependentsRead
}) {
  const facet = review.dependents
  const unlisted =
    "Views, routines and triggers that use this object are not listed."
  if (facet.type === "notApplicable") return <p>{unlisted}</p>
  if (facet.type === "notReported") {
    return (
      <>
        <p>Dependents are not reported by this connection.</p>
        <p>{unlisted}</p>
      </>
    )
  }
  let known: React.ReactNode
  if (dependents.kind === "reading") {
    known = (
      <p role="status" className="flex items-center gap-1.5">
        <Spinner aria-hidden className="size-3" />
        Reading the foreign keys that reference it…
      </p>
    )
  } else if (dependents.kind === "failed") {
    // A failed read is said, and never shown as « none ».
    known = (
      <p className="text-destructive">
        The foreign keys that reference it could not be read:{" "}
        {dependents.message}
      </p>
    )
  } else if (facet.value === null) {
    known = <p>The foreign keys that reference it have not been read.</p>
  } else if (facet.value.length === 0) {
    known = <p>No foreign key references it.</p>
  } else {
    known = <KeyList keys={facet.value} />
  }
  return (
    <>
      {known}
      <p>{unlisted}</p>
    </>
  )
}

function KeyList({ keys }: { keys: Array<IncomingKeyRow> }) {
  return (
    <div>
      <p>
        Referenced by{" "}
        {keys.length === 1 ? "one foreign key" : `${keys.length} foreign keys`}:
      </p>
      <ul className="mt-1 flex max-h-24 flex-col gap-0.5 overflow-auto">
        {keys.map((key, index) => (
          <li key={index} className="font-mono wrap-anywhere text-foreground">
            <bdi>{sourceLabel(key.source)}</bdi> ({key.key.fields.join(", ")})
          </li>
        ))}
      </ul>
    </div>
  )
}

function PhaseNotice({ phase }: { phase: ReviewPhase }) {
  switch (phase.kind) {
    case "idle":
    case "running":
      return null
    case "denied":
      return (
        <Alert variant="destructive">
          <AlertTitle>Not run</AlertTitle>
          <AlertDescription data-selectable className="wrap-anywhere">
            {phase.reason}
          </AlertDescription>
        </Alert>
      )
    case "failed":
      return (
        <Alert variant="destructive">
          <AlertTitle>The server refused it</AlertTitle>
          <AlertDescription data-selectable className="wrap-anywhere">
            {phase.message}
          </AlertDescription>
        </Alert>
      )
    case "ambiguous":
      return (
        <Alert variant="destructive">
          <AlertTitle>Outcome unknown</AlertTitle>
          <AlertDescription data-selectable className="wrap-anywhere">
            <p>{AMBIGUOUS}</p>
            <p>{phase.message}</p>
          </AlertDescription>
        </Alert>
      )
    case "notSent":
      return (
        <Alert>
          <AlertTitle>Nothing was sent</AlertTitle>
          <AlertDescription data-selectable className="wrap-anywhere">
            {phase.message}
          </AlertDescription>
        </Alert>
      )
  }
}
