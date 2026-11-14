import { ConnectionForm } from "./connection-form"
import type { ConnectionPrefill } from "./connection-form"
import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import type { DriverChoice, ConnectionDraft } from "@/lib/ipc/types"
import type { ConnectionDetails } from "@/lib/ipc/settings"

export interface DatabaseConnectionDialogProps {
  database: string
  driver: DriverChoice
  prefill: ConnectionPrefill
  candidates: Array<ConnectionDetails>
  submitting: boolean
  error: BackendFailure | null
  saved: boolean
  onSubmit: (draft: ConnectionDraft) => void
  onChoose: (id: string) => void
  onFresh: () => void
  onClose: () => void
  onCancel: () => void
}

/** A new base reviews its own account and protections; server and base stay fixed. */
export function DatabaseConnectionDialog(p: DatabaseConnectionDialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) p.submitting ? p.onCancel() : p.onClose()
      }}
    >
      <DialogContent
        className="flex max-h-[90vh] flex-col overflow-y-auto sm:max-w-2xl"
        showCloseButton={!p.submitting}
      >
        <DialogHeader>
          <DialogTitle>
            Open database · <bdi>{p.database}</bdi>
          </DialogTitle>
          <DialogDescription>
            Choose a saved connection or review a new, independent account. No
            SQL draft is copied or executed.
          </DialogDescription>
        </DialogHeader>
        {p.candidates.length > 0 ? (
          <div className="flex flex-col gap-2">
            {p.candidates.map((candidate) => (
              <Button
                variant="outline"
                key={candidate.id}
                disabled={p.submitting}
                onClick={() => p.onChoose(candidate.id)}
              >
                <bdi>{candidate.name}</bdi> · <bdi>{candidate.values.user}</bdi>{" "}
                · {candidate.environment}
              </Button>
            ))}
            <Button variant="ghost" disabled={p.submitting} onClick={p.onFresh}>
              Create another connection
            </Button>
            {p.submitting && (
              <Button variant="outline" onClick={p.onCancel}>
                Cancel opening
              </Button>
            )}
          </div>
        ) : p.saved ? (
          <>
            <p>
              The connection was saved. Open it from the explorer to retry; edit
              its credentials in connection settings if needed.
            </p>
            <Button variant="outline" onClick={p.onClose}>
              Return to explorer
            </Button>
          </>
        ) : (
          <ConnectionForm
            driver={p.driver}
            prefill={p.prefill}
            lockedDestination
            submitting={p.submitting}
            onSubmit={p.onSubmit}
            onCancel={p.onClose}
            onAbort={p.onCancel}
            error={p.error}
          />
        )}
        {p.error && (p.saved || p.candidates.length > 0) && (
          <BackendErrorAlert title="Database not opened" error={p.error} />
        )}
      </DialogContent>
    </Dialog>
  )
}
