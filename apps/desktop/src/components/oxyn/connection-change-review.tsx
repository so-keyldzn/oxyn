import * as React from "react"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
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
import { Spinner } from "@/components/ui/spinner"
import type { Environment } from "@/lib/ipc/types"

export interface PendingConnectionChange {
  command: string
  kind: "update" | "delete"
  reason: string
  /** The name the user knows, before an edit renamed it. */
  connectionName: string
  environment: Environment
}

/**
 * The policy's review of an edit or a deletion on a connection it protects
 * (I-02): a production connection is changed only on a decision that names it.
 *
 * `Cancel` takes the initial focus, Enter alone decides nothing, Escape
 * rejects — the same guard as the review of a write.
 */
export function ConnectionChangeReview({
  change,
  deciding = false,
  onDecide,
}: {
  change: PendingConnectionChange | null
  deciding?: boolean
  onDecide: (approved: boolean) => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const name = change?.connectionName ?? ""
  const deleting = change?.kind === "delete"

  return (
    <AlertDialog
      open={change !== null}
      onOpenChange={(open) => {
        if (!open && !deciding) onDecide(false)
      }}
    >
      <AlertDialogContent
        initialFocus={cancelRef}
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
          <div className="flex items-center gap-2">
            <AlertDialogTitle>
              {deleting ? "Confirm the deletion" : "Confirm the changes"}
            </AlertDialogTitle>
            {change ? (
              <EnvironmentBadge environment={change.environment} />
            ) : null}
          </div>
          <AlertDialogDescription>
            <span data-selectable>{change?.reason}</span>{" "}
            {deleting
              ? "The saved connection and its keyring secrets are removed:"
              : "The saved configuration changes for"}{" "}
            <strong className="font-medium text-foreground">
              <bdi>{name}</bdi>
            </strong>
            .
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel ref={cancelRef} disabled={deciding}>
            Cancel
          </AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={deciding}
            onClick={() => onDecide(true)}
            // The name is whatever the user typed: bounded here, in full above.
            title={deleting ? `Delete ${name}` : `Save ${name}`}
            className="max-w-full"
          >
            {deciding ? <Spinner data-icon="inline-start" /> : null}
            <span className="min-w-0 truncate">
              {deleting ? "Delete" : "Save"} <bdi>{name}</bdi>
            </span>
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
