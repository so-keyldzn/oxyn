import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, Copy01Icon } from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { SqlEditor } from "@/components/oxyn/sql-editor"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Spinner } from "@/components/ui/spinner"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import type {
  DocumentEntry,
  DocumentView,
  HistoryDetail,
} from "@/lib/ipc/library"

export type LibraryEntryState =
  | { status: "loading" }
  | { status: "error"; error: BackendFailure }
  | { status: "history"; entry: HistoryDetail }
  | { status: "saved"; document: DocumentView; entry: DocumentEntry }

const ignore = () => undefined

function heading(state: LibraryEntryState) {
  switch (state.status) {
    case "history":
      return `History entry #${state.entry.id}`
    case "saved":
      return state.entry.title === "" ? "Untitled query" : state.entry.title
    default:
      return "Library entry"
  }
}

/**
 * The full text of a history entry or a saved query, read only (UX-SPEC
 * « Consultation locale des requêtes »). It opens beside the console and
 * touches neither its draft nor its result: selection and copy work, typing
 * and ⌘↵ do nothing.
 *
 * A write that needs inspection stays readable with its warning, and offers
 * no editable copy (I-13): replaying it could apply it twice.
 */
export function LibraryEntryView({
  open,
  state,
  driver,
  destination,
  onOpenCopy,
  onRetry,
  onClose,
}: {
  open: boolean
  state: LibraryEntryState
  /** The SQL dialect the text is highlighted with. */
  driver: string
  /** The connection a copy opens on, named before the click. */
  destination: string
  onOpenCopy: () => void
  /** Offered only for an error the backend declared transient. */
  onRetry: () => void
  onClose: () => void
}) {
  const [shown, setShown] = React.useState<"saved" | "working">("saved")
  const needsInspection =
    state.status === "history" && state.entry.needsInspection
  const working =
    state.status === "saved" &&
    state.document.savedText !== null &&
    state.document.savedText !== state.document.text
  const text =
    state.status === "history"
      ? state.entry.statement
      : state.status === "saved"
        ? shown === "working" || state.document.savedText === null
          ? state.document.text
          : state.document.savedText
        : ""
  const fromAgent =
    (state.status === "history" && state.entry.fromAgent) ||
    (state.status === "saved" && state.document.fromAgent)

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          setShown("saved")
          onClose()
        }
      }}
    >
      <DialogContent className="flex max-h-[80vh] flex-col sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="wrap-anywhere">
            <bdi>{heading(state)}</bdi> · Read only
          </DialogTitle>
          <DialogDescription className="wrap-anywhere">
            {state.status === "history" ? (
              <>
                <bdi>
                  {state.entry.connectionName ?? "Connection unavailable"}
                </bdi>{" "}
                · {state.entry.status}
              </>
            ) : state.status === "saved" ? (
              <bdi>{state.entry.connectionName ?? "No connection"}</bdi>
            ) : (
              "Reading the full text. Nothing is run."
            )}
          </DialogDescription>
          {fromAgent ? (
            <div>
              <Badge variant="secondary">Written by an agent</Badge>
            </div>
          ) : null}
        </DialogHeader>

        {needsInspection ? (
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>This write needs inspection</AlertTitle>
            <AlertDescription>
              It may have reached the server. Inspect the server state, then
              mark it reconciled from the history. It is never replayed, and no
              editable copy is offered.
            </AlertDescription>
          </Alert>
        ) : null}

        {working ? (
          <ToggleGroup
            value={[shown]}
            onValueChange={(value) => {
              const next = value[0]
              if (next === "saved" || next === "working") setShown(next)
            }}
            variant="outline"
            size="sm"
          >
            <ToggleGroupItem value="saved">Saved copy</ToggleGroupItem>
            <ToggleGroupItem value="working">Working copy</ToggleGroupItem>
          </ToggleGroup>
        ) : null}

        {state.status === "loading" ? (
          <p
            role="status"
            className="flex items-center gap-2 text-sm text-muted-foreground"
          >
            <Spinner />
            Reading the full text…
          </p>
        ) : state.status === "error" ? (
          <BackendErrorAlert
            title="The full text could not be read."
            error={state.error}
            onRetry={onRetry}
            nextStep="Close this view and refresh the library."
          />
        ) : (
          <div className="h-[50vh] min-h-40 overflow-hidden rounded-md border">
            <SqlEditor
              value={text}
              onChange={ignore}
              onRun={ignore}
              driver={driver}
              readOnly
            />
          </div>
        )}

        <DialogFooter>
          <DialogClose render={<Button variant="outline" />}>Close</DialogClose>
          {state.status === "history" || state.status === "saved" ? (
            needsInspection ? null : (
              <Button onClick={onOpenCopy}>
                <HugeiconsIcon
                  icon={Copy01Icon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Open copy in <bdi>{destination}</bdi>
              </Button>
            )
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
