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
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import type { ConnectionSummary } from "@/lib/ipc/settings"
import type { SavedConnection } from "@/lib/ipc/types"
import { TextInput } from "./text-field"

/**
 * Deleting a saved connection, built not to be confirmed by reflex
 * (docs/UX-SPEC.md, « Les opérations destructrices »).
 *
 * The dialog names the connection and the action only unlocks once that name
 * is typed; `Cancel` takes the initial focus and Enter alone deletes nothing.
 * What is deleted is Oxyn's saved configuration and its keyring secrets —
 * never the database.
 */
export function DeleteConnectionDialog<
  T extends SavedConnection & Partial<ConnectionSummary>,
>({
  connection,
  deleting = false,
  error,
  onCancel,
  onConfirm,
}: {
  connection: T | null
  deleting?: boolean
  error?: string | null
  onCancel: () => void
  onConfirm: (connection: T) => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const nameId = React.useId()
  const [typed, setTyped] = React.useState("")
  const name = connection?.name ?? ""
  const matches = connection !== null && typed === name

  React.useEffect(() => {
    setTyped("")
  }, [connection?.id])

  return (
    <AlertDialog
      open={connection !== null}
      onOpenChange={(open) => {
        if (!open && !deleting) onCancel()
      }}
    >
      <AlertDialogContent
        // A grid track sized by its content would let a long name widen the
        // footer past the frame.
        className="grid-cols-1"
        initialFocus={cancelRef}
        onKeyDown={(event) => {
          // Enter in the name field must not reach the destructive action.
          if (
            event.key === "Enter" &&
            !(event.target instanceof HTMLButtonElement)
          ) {
            event.preventDefault()
          }
        }}
      >
        <AlertDialogHeader>
          <div className="flex w-full min-w-0 items-center gap-2">
            <AlertDialogTitle className="min-w-0 truncate">
              Delete <bdi>{name}</bdi>?
            </AlertDialogTitle>
            {connection ? (
              <EnvironmentBadge environment={connection.environment} />
            ) : null}
          </div>
          <AlertDialogDescription className="wrap-anywhere">
            The saved connection{" "}
            <strong className="font-medium text-foreground">
              <bdi>{name}</bdi>
            </strong>
            {connection?.location ? (
              <>
                {" "}
                (<bdi className="font-mono">{connection.location}</bdi>)
              </>
            ) : null}{" "}
            and its secrets in the system keyring are removed from this
            workspace. Its open sessions are closed. The database itself is not
            touched.
          </AlertDialogDescription>
          {/* Said before, not discovered after: what was asked about this
              connection stays in the workspace file (docs/UX-SPEC.md). */}
          <p
            data-slot="delete-connection-conversations"
            className="min-w-0 text-sm wrap-anywhere text-muted-foreground"
          >
            Its assistant conversations are kept in this workspace, under the
            name <bdi>{name}</bdi>, and stay listed, read-only, in the assistant
            panel.
          </p>
        </AlertDialogHeader>

        <Field>
          <FieldLabel htmlFor={nameId}>
            Type the connection name to confirm
          </FieldLabel>
          <TextInput
            id={nameId}
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
            autoComplete="off"
            disabled={deleting}
          />
          <FieldDescription>
            Exactly <bdi className="font-mono break-all">{name}</bdi>.
          </FieldDescription>
        </Field>

        {error ? (
          <Alert variant="destructive">
            <AlertTitle>Connection not deleted</AlertTitle>
            <AlertDescription data-selectable className="wrap-anywhere">
              {error}
            </AlertDescription>
          </Alert>
        ) : null}

        <AlertDialogFooter>
          <AlertDialogCancel ref={cancelRef} disabled={deleting}>
            Cancel
          </AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={!matches || deleting}
            onClick={() => {
              if (connection && matches) onConfirm(connection)
            }}
            // The name is whatever the user typed: bounded here, in full above.
            title={`Delete ${name}`}
            className="min-w-0 shrink"
          >
            {deleting ? <Spinner data-icon="inline-start" /> : null}
            <span className="min-w-0 truncate">
              Delete <bdi>{name}</bdi>
            </span>
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
