import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { createStore, useStore } from "@tanstack/react-store"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon } from "@hugeicons/core-free-icons"

import { ConnectionChangeReview } from "@/components/oxyn/connection-change-review"
import type { PendingConnectionChange } from "@/components/oxyn/connection-change-review"
import { ConnectionForm } from "@/components/oxyn/connection-form"
import { ConnectionManager } from "@/components/oxyn/connection-manager"
import { DiscardChangesDialog } from "@/components/oxyn/discard-changes-dialog"
import { DeleteConnectionDialog } from "@/components/oxyn/delete-connection-dialog"
import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Skeleton } from "@/components/ui/skeleton"
import { copyConnection } from "@/features/connections/copy-connection"
import { useRefreshCatalogOf } from "@/features/connections/refresh-catalog"
import { useTypedSecrets } from "@/features/connections/typed-secrets"
import { LIBRARY_QUERY_KEY } from "@/features/library/library-refresh"
import { closeConnection, session } from "@/features/session"
import type { WithoutSecrets } from "@/features/connections/typed-secrets"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { settingsBackend } from "@/lib/ipc/settings"
import type { ConnectionChange, ConnectionSummary } from "@/lib/ipc/settings"
import type { ConnectionDraft, OpenConnection } from "@/lib/ipc/types"

/** What another screen asks of this panel for one saved connection. */
export interface ConnectionRequest {
  connection: string
  /** `environment` is an edit that starts on the environment choice. */
  intent: "edit" | "environment" | "delete"
}

const connectionRequest = createStore<ConnectionRequest | null>(null)

/**
 * Asks this panel to edit or delete a connection, through its own form and
 * dialogs — a change of environment still meets ADR-0037's native dialog.
 * The caller opens the settings on `connections`; the panel takes the
 * request once the list is read, and drops it if the connection is gone.
 */
export function requestConnectionChange(request: ConnectionRequest) {
  connectionRequest.setState(() => request)
}

function failureOf(error: unknown): BackendFailure | null {
  if (!error) return null
  return error instanceof BackendError
    ? { message: error.message, retryable: error.retryable }
    : { message: String(error), retryable: false }
}

function messageOf(error: unknown) {
  return failureOf(error)?.message ?? null
}

/**
 * Editing and deleting saved connections, from the settings dialog.
 *
 * After an edit of the open connection, its marking is read again and handed
 * to `onOpenConnectionChanged`: the workspace holds a copy taken when the
 * session opened, and the assistant would otherwise keep the old privacy tier
 * (I-04).
 */
export function ConnectionsSettings({
  openConnections,
  onOpenConnectionChanged,
  onUnsavedEditChange,
}: {
  /** Every connection with a workspace in the window, shown or hidden. */
  openConnections: Array<OpenConnection>
  onOpenConnectionChanged?: (open: OpenConnection) => void
  /** Typed values of an edit that closing the panel would lose. */
  onUnsavedEditChange?: (unsaved: boolean) => void
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const refreshCatalogOf = useRefreshCatalogOf()
  const typedSecrets = useTypedSecrets()
  const connections = useQuery({
    queryKey: ["connections"],
    queryFn: settingsBackend.listConnections,
  })
  const drivers = useQuery({
    queryKey: ["drivers"],
    queryFn: backend.listDrivers,
  })
  const [editing, setEditing] = React.useState<ConnectionSummary | null>(null)
  const [deleting, setDeleting] = React.useState<ConnectionSummary | null>(null)
  const [review, setReview] = React.useState<PendingConnectionChange | null>(
    null
  )
  const [secretsError, setSecretsError] = React.useState<string | null>(null)
  const [dirty, setDirty] = React.useState(false)
  const [confirmingCancel, setConfirmingCancel] = React.useState(false)
  const [focusEnvironment, setFocusEnvironment] = React.useState(false)
  const formRef = React.useRef<HTMLDivElement>(null)
  const unsaved = editing !== null && dirty

  React.useEffect(() => {
    onUnsavedEditChange?.(unsaved)
  }, [unsaved, onUnsavedEditChange])
  // Leaving the panel (another section, the dialog closed) ends the edit.
  React.useEffect(
    () => () => onUnsavedEditChange?.(false),
    [onUnsavedEditChange]
  )

  const stopEditing = () => {
    update.reset()
    setDirty(false)
    setConfirmingCancel(false)
    setEditing(null)
  }

  const details = useQuery({
    queryKey: ["connection-details", editing?.id],
    queryFn: () => settingsBackend.connectionDetails(editing!.id),
    enabled: editing !== null,
    // Never kept: the next edit reads what is saved then.
    gcTime: 0,
  })

  const settle = async (
    change: ConnectionChange | null,
    target: ConnectionSummary
  ) => {
    if (change === null) {
      // Rejected, in the review or in the host's native dialog: nothing
      // changed. A deletion ends there; an edit stays open.
      setReview(null)
      setDeleting(null)
      return
    }
    if (change.type === "approval") {
      setReview({
        command: change.command,
        kind: deleting ? "delete" : "update",
        reason: change.reason,
        connectionName: change.preview?.connection ?? target.name,
        environment: target.environment,
      })
      return
    }
    setReview(null)
    await queryClient.invalidateQueries({ queryKey: ["connections"] })
    // The library's connection filter names connections too: a renamed or
    // removed one must not keep its old label there.
    void queryClient.invalidateQueries({
      queryKey: [...LIBRARY_QUERY_KEY, "history-connections"],
    })
    if (change.type === "deleted") {
      setDeleting(null)
      return
    }
    setDirty(false)
    setEditing(null)
    setSecretsError(change.secretsError)
    // A hidden workspace gets it too: it would show the old tier when it
    // comes back, and its assistant would answer under it (I-04).
    const openConnection = openConnections.find(
      (open) => open.connection === target.id
    )
    if (openConnection) {
      const marking = await settingsBackend.connectionMarking(target.id)
      onOpenConnectionChanged?.({
        ...openConnection,
        name: marking.name,
        environment: marking.environment,
        readOnly: marking.readOnly,
        privacyTier: marking.privacyTier,
      })
    }
  }

  const update = useMutation({
    mutationFn: ({
      target,
      result,
    }: {
      target: ConnectionSummary
      result: WithoutSecrets<ConnectionDraft>
    }) =>
      settingsBackend.updateConnection(newCommandId(), target.id, {
        name: result.name,
        environment: result.environment,
        privacyTier: result.privacyTier,
        readOnly: result.readOnly,
        values: result.values,
        secrets: typedSecrets.take(result),
      }),
    onSuccess: (change, { target }) => settle(change, target),
  })

  const remove = useMutation({
    mutationFn: (target: ConnectionSummary) =>
      settingsBackend.deleteConnection(newCommandId(), target.id),
    onSuccess: (change, target) => settle(change, target),
  })

  const decide = useMutation({
    mutationFn: ({ approved }: { approved: boolean }) =>
      settingsBackend.decideConnectionChange(review!.command, approved),
    onSuccess: (change) => {
      const target = deleting ?? editing
      if (target) return settle(change, target)
      setReview(null)
    },
    onError: () => setReview(null),
  })

  const startEditing = (connection: ConnectionSummary, environment = false) => {
    setSecretsError(null)
    update.reset()
    setFocusEnvironment(environment)
    setEditing(connection)
  }
  const startDeleting = (connection: ConnectionSummary) => {
    // As the list's button: an open connection is disconnected first.
    if (openConnections.some((open) => open.connection === connection.id))
      return
    remove.reset()
    setDeleting(connection)
  }

  // A request from another screen — the start screen's context menu — goes
  // through the same form and dialogs as a click here. An edit with typed
  // values is never replaced by it.
  const request = useStore(connectionRequest)
  React.useEffect(() => {
    if (request === null || connections.data === undefined) return
    connectionRequest.setState(() => null)
    const target = connections.data.find(
      (connection) => connection.id === request.connection
    )
    if (!target || unsaved || review !== null) return
    if (request.intent === "delete") startDeleting(target)
    else startEditing(target, request.intent === "environment")
    // Only the request and the list: the closures are fresh each render.
  }, [request, connections.data])

  // `Change environment…` lands on the chosen environment rather than on the
  // name, which the form focuses first.
  const formReady = details.data !== undefined && drivers.data !== undefined
  React.useEffect(() => {
    if (!focusEnvironment || !formReady) return
    const frame = requestAnimationFrame(() => {
      const chosen = formRef.current?.querySelector<HTMLElement>(
        '[data-slot="environment-picker"] [role="radio"][aria-checked="true"]'
      )
      chosen?.focus()
      chosen?.scrollIntoView({ block: "center" })
      setFocusEnvironment(false)
    })
    return () => cancelAnimationFrame(frame)
  }, [focusEnvironment, formReady])

  if (editing) {
    const driver = drivers.data?.find((choice) => choice.id === editing.driver)
    const failure = failureOf(details.error ?? drivers.error)
    return (
      <div ref={formRef} className="flex flex-col gap-4">
        <h3 className="truncate font-medium">
          Edit <bdi title={editing.name}>{editing.name}</bdi>
        </h3>
        {failure ? (
          <BackendErrorAlert
            title="Cannot edit this connection"
            error={failure}
            onRetry={() => void details.refetch()}
            nextStep="It may have been deleted elsewhere: go back to the list."
          />
        ) : details.data && driver ? (
          <ConnectionForm
            key={editing.id}
            driver={driver}
            existing={details.data}
            submitting={update.isPending || decide.isPending}
            error={failureOf(update.error ?? decide.error)}
            onSubmit={(result) =>
              update.mutate({
                target: editing,
                result: typedSecrets.hold(result),
              })
            }
            onCancel={() => {
              if (dirty) setConfirmingCancel(true)
              else stopEditing()
            }}
            onDirtyChange={setDirty}
          />
        ) : drivers.isSuccess && !driver ? (
          <Alert variant="destructive">
            <AlertTitle>Driver unavailable</AlertTitle>
            <AlertDescription>
              This build no longer offers the {editing.driver} driver.
            </AlertDescription>
          </Alert>
        ) : (
          <div className="flex flex-col gap-2" aria-busy="true">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-24 w-full" />
          </div>
        )}
        <DiscardChangesDialog
          open={confirmingCancel}
          what="the changes to this connection"
          onKeep={() => setConfirmingCancel(false)}
          onDiscard={stopEditing}
        />
        <ConnectionChangeReview
          change={review}
          deciding={decide.isPending}
          onDecide={(approved) => decide.mutate({ approved })}
        />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      {secretsError ? (
        <Alert>
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>Saved, but the secrets were not written</AlertTitle>
          <AlertDescription data-selectable>{secretsError}</AlertDescription>
        </Alert>
      ) : null}
      <ConnectionManager
        connections={connections.data}
        error={failureOf(connections.error)}
        openConnectionIds={openConnections.map((open) => open.connection)}
        onEdit={(connection) => startEditing(connection)}
        onDelete={startDeleting}
        onRetry={() => void connections.refetch()}
        menuActions={(connection) => {
          const open = openConnections.find(
            (held) => held.connection === connection.id
          )
          return {
            // The start screen's `Disconnect`: drafts written, then the
            // sessions closed (`WorkspaceHost`). The shown workspace gives
            // way to the start screen, as its own `Disconnect` does.
            disconnect: open
              ? () => {
                  const shown =
                    session.state.open?.connection === open.connection
                  closeConnection(open.connection)
                  if (shown) void navigate({ to: "/" })
                }
              : undefined,
            refreshCatalog: open
              ? () => void refreshCatalogOf(open)
              : undefined,
            changeEnvironment: () => startEditing(connection, true),
            copy: () => void copyConnection(connection),
          }
        }}
      />
      <DeleteConnectionDialog
        connection={review ? null : deleting}
        deleting={remove.isPending}
        error={messageOf(remove.error ?? decide.error)}
        onCancel={() => setDeleting(null)}
        onConfirm={(connection) => remove.mutate(connection)}
      />
      <ConnectionChangeReview
        change={review}
        deciding={decide.isPending}
        onDecide={(approved) => decide.mutate({ approved })}
      />
    </div>
  )
}
