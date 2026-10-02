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

const ACTIONS = {
  create: {
    title: "Confirm the new connection",
    lead: "Approving saves",
    tail: " with this marking, then connects to it. No statement of yours is run.",
    verb: "Save and connect to",
  },
  update: {
    title: "Confirm the changes",
    lead: "The saved configuration changes for",
    tail: ".",
    verb: "Save",
  },
  delete: {
    title: "Confirm the deletion",
    lead: "The saved connection and its keyring secrets are removed:",
    tail: ".",
    verb: "Delete",
  },
} as const

export interface PendingConnectionChange {
  command: string
  kind: "create" | "update" | "delete"
  reason: string
  /** The name the user knows, before an edit renamed it. */
  connectionName: string
  environment: Environment
}

/**
 * The policy's review of a creation, an edit or a deletion on a connection it
 * protects (I-02): a production connection is saved, changed or removed only
 * on a decision that names it. What is approved is a saved profile, not a
 * statement: nothing here speaks of rows or SQL.
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
  const kind = change?.kind ?? "update"
  const action = ACTIONS[kind]

  return (
    <AlertDialog
      open={change !== null}
      onOpenChange={(open) => {
        if (!open && !deciding) onDecide(false)
      }}
    >
      <AlertDialogContent
        // A grid track sized by its content would let a long name widen the
        // footer past the frame.
        className="grid-cols-1"
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
          <div className="flex flex-wrap items-center gap-2">
            <AlertDialogTitle>{action.title}</AlertDialogTitle>
            {change ? (
              <EnvironmentBadge environment={change.environment} />
            ) : null}
          </div>
          <AlertDialogDescription className="wrap-anywhere">
            <span data-selectable>{change?.reason}</span> {action.lead}{" "}
            <strong className="font-medium text-foreground">
              <bdi>{name}</bdi>
            </strong>
            {action.tail}
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
            title={`${action.verb} ${name}`}
            className="min-w-0 shrink"
          >
            {deciding ? <Spinner data-icon="inline-start" /> : null}
            <span className="min-w-0 truncate">
              {action.verb} <bdi>{name}</bdi>
            </span>
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
