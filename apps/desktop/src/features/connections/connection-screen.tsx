import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useStore } from "@tanstack/react-store"
import { open as openDialog } from "@tauri-apps/plugin-dialog"

import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { toast } from "@/components/ui/toast"
import { ConnectionScreenView } from "@/features/connections/connection-screen-view"
import type { PendingConnectionApproval } from "@/features/connections/connection-screen-view"
import type { ConnectionPrefill } from "@/components/oxyn/connection-form"
import { copyConnection } from "@/features/connections/copy-connection"
import { duplicatePrefill } from "@/features/connections/duplicate-connection"
import {
  clearConnectionOffer,
  connectionOffer,
} from "@/features/connections/connection-offer"
import { useTypedSecrets } from "@/features/connections/typed-secrets"
import type { WithoutSecrets } from "@/features/connections/typed-secrets"
import { closeConnection, openConnection, session } from "@/features/session"
import { requestConnectionChange } from "@/features/settings/connections-settings"
import type { ConnectionRequest } from "@/features/settings/connections-settings"
import { openSettings } from "@/features/settings/settings-dialog"
import { actionSources } from "@/lib/actions/context"
import type { ConnectionMenuActions } from "@/lib/actions/targets"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import { settingsBackend } from "@/lib/ipc/settings"
import type { ConnectionSummary } from "@/lib/ipc/settings"
import type {
  ConnectResponse,
  ConnectionDraft,
  DriverChoice,
  OpenConnection,
} from "@/lib/ipc/types"

function asOpen(response: ConnectResponse): OpenConnection | null {
  if (response.type !== "open") return null
  const { type: _type, ...open } = response
  return open
}

function failureOf(error: unknown): BackendFailure | null {
  if (!error) return null
  if (error instanceof BackendError)
    return { message: error.message, retryable: error.retryable }
  return { message: String(error), retryable: false }
}

/** Past this, the workspace did not show: the console is not opened late. */
const WORKSPACE_SHOWN_WITHIN_MS = 2_000

/**
 * Shows the workspace left open, then opens a console in it — the registry's
 * `New console`, through the actions it publishes. Hidden, it publishes
 * none: the console waits until it is on screen, and never opens later than
 * the user could connect it to this click.
 */
function openConsoleInWorkspace(show: () => void) {
  const subscription = actionSources.subscribe(() => {
    const workspace = actionSources.state.sources.workspace
    if (!workspace) return
    stop()
    workspace.actions.openConsole()
  })
  const timer = setTimeout(() => stop(), WORKSPACE_SHOWN_WITHIN_MS)
  function stop() {
    clearTimeout(timer)
    subscription.unsubscribe()
  }
  show()
}

/**
 * The start screen's link to the backend. The view is
 * `ConnectionScreenView`; everything that reaches IPC lives here.
 *
 * Opening is cancellable: the command id given to `connect`/`reconnect` is
 * kept, and Cancel sends it to `cancel`, which reaches the server (UX-SPEC
 * « Annulation »). An answer that arrives after the user cancelled is not
 * entered: a session it opened anyway is closed rather than shown.
 */
export function ConnectionScreen() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  // Keeps the entered secrets out of `connect`'s variables, so neither the
  // MutationCache nor the React Query Devtools ever retain them (I-03).
  const typedSecrets = useTypedSecrets()
  const leftOpen = useStore(session, (state) => state.open)
  const drivers = useQuery({
    queryKey: ["drivers"],
    queryFn: backend.listDrivers,
  })
  const saved = useQuery({
    queryKey: ["connections"],
    queryFn: settingsBackend.listConnections,
  })
  const [driver, setDriver] = React.useState<DriverChoice | null>(null)
  const [duplicate, setDuplicate] = React.useState<{
    of: string
    prefill: ConnectionPrefill
  } | null>(null)
  // A dropped database file: its driver's form, the file already in it.
  // Taken once, so coming back to this screen later does not replay it.
  const offer = useStore(connectionOffer)
  const [prefill, setPrefill] = React.useState<{
    key: number
    values: Record<string, string>
  } | null>(null)
  React.useEffect(() => {
    if (!offer || !drivers.data) return
    clearConnectionOffer()
    const choice = drivers.data.find((known) => known.id === offer.driver)
    // Rust offers only a registered driver; a list read before a change of
    // build would still not have it, and then nothing is offered.
    if (!choice) return
    setDuplicate(null)
    setPrefill({ key: offer.id, values: { [offer.field]: offer.path } })
    setDriver(choice)
  }, [offer, drivers.data])
  const [approval, setApproval] =
    React.useState<PendingConnectionApproval | null>(null)
  const [cancelling, setCancelling] = React.useState(false)
  const inFlight = React.useRef<string | null>(null)
  const cancelled = React.useRef(new Set<string>())

  const enter = (open: OpenConnection) => {
    openConnection(open)
    void queryClient.invalidateQueries({ queryKey: ["connections"] })
    void navigate({ to: "/workspace" })
  }

  /** Runs a cancellable opening; resolves to `null` when the user cancelled. */
  const cancellable = async <T,>(run: (commandId: string) => Promise<T>) => {
    const commandId = newCommandId()
    inFlight.current = commandId
    try {
      const result = await run(commandId)
      return cancelled.current.has(commandId) ? null : result
    } catch (error) {
      if (cancelled.current.has(commandId)) return null
      throw error
    } finally {
      if (inFlight.current === commandId) inFlight.current = null
      setCancelling(false)
    }
  }

  const discardLate = (response: ConnectResponse | OpenConnection | null) => {
    const open =
      response && "type" in response ? asOpen(response) : (response ?? null)
    // `disconnect` closes every session of the connection: never the one the
    // workspace left open is still using.
    if (open && open.connection !== session.state.open?.connection)
      void backend.disconnect(open.connection).catch(() => undefined)
  }

  const connect = useMutation({
    mutationFn: async (draft: WithoutSecrets<ConnectionDraft>) => {
      const held: { late: ConnectResponse | null } = { late: null }
      const response = await cancellable(async (commandId) => {
        held.late = await backend.connect(commandId, {
          ...draft,
          secrets: typedSecrets.take(draft),
        })
        return held.late
      })
      if (response === null) discardLate(held.late)
      return response
    },
    onSuccess: (response, draft) => {
      if (response === null) return
      if (response.type === "approval") {
        setApproval({
          command: response.command,
          reason: response.reason,
          preview: response.preview,
          name: draft.name,
          environment: draft.environment,
        })
        return
      }
      const open = asOpen(response)
      if (open) enter(open)
    },
  })

  // Cancellable like an opening, and through the same id: Esc and Cancel
  // reach the server whichever of the two is in flight. A cancelled test
  // answers `null`, and the status bar goes back to « Not tested ».
  const test = useMutation({
    mutationFn: (draft: WithoutSecrets<ConnectionDraft>) =>
      cancellable((commandId) =>
        backend.testConnection(commandId, {
          ...draft,
          secrets: typedSecrets.take(draft),
        })
      ),
  })

  const reconnect = useMutation({
    mutationFn: async (connection: ConnectionSummary) => {
      const held: { late: OpenConnection | null } = { late: null }
      const open = await cancellable(async (commandId) => {
        held.late = await backend.reconnect(commandId, connection.id)
        return held.late
      })
      if (open === null) discardLate(held.late)
      return open
    },
    onSuccess: (open) => {
      if (open) enter(open)
    },
  })

  const decide = useMutation({
    mutationFn: ({
      command,
      approved,
    }: {
      command: string
      approved: boolean
    }) => backend.decideConnection(command, approved),
    onSuccess: (response) => {
      setApproval(null)
      const open = response ? asOpen(response) : null
      if (open) enter(open)
    },
    onError: () => setApproval(null),
  })

  const cancelOpening = () => {
    const commandId = inFlight.current
    if (!commandId || cancelled.current.has(commandId)) return
    cancelled.current.add(commandId)
    setCancelling(true)
    void backend.cancel(commandId).catch(() => undefined)
  }

  const idle = () =>
    !(reconnect.isPending || connect.isPending || test.isPending)

  const connectionMenu = (
    connection: ConnectionSummary
  ): ConnectionMenuActions => {
    const open = leftOpen?.connection === connection.id ? leftOpen : null
    const settings = (intent: ConnectionRequest["intent"]) => () => {
      requestConnectionChange({ connection: connection.id, intent })
      openSettings("connections")
    }
    return {
      // The workspace's own `Disconnect`: its drafts are written, then its
      // sessions close (`WorkspaceHost`).
      disconnect: open ? closeConnection : undefined,
      // Closed, opening is the new console: a workspace starts on one.
      newConsole: open
        ? () =>
            openConsoleInWorkspace(() => void navigate({ to: "/workspace" }))
        : () => {
            if (idle()) reconnect.mutate(connection)
          },
      refreshCatalog: open ? () => void refreshCatalogOf(open) : undefined,
      edit: settings("edit"),
      duplicate: () => void startDuplicate(connection),
      changeEnvironment: settings("environment"),
      copy: () => void copyConnection(connection),
      // As in the settings: the connection in use is left before deletion.
      delete: open ? undefined : settings("delete"),
    }
  }

  // The new connection form, filled with the source's non-secret parameters:
  // nothing is saved before the user connects, and the secrets are typed.
  const startDuplicate = async (connection: ConnectionSummary) => {
    if (!idle()) return
    const choice = drivers.data?.find((item) => item.id === connection.driver)
    if (!choice) {
      toast.add({
        title: "Cannot duplicate this connection",
        description: `This build does not offer the ${connection.driverName} driver.`,
        type: "error",
      })
      return
    }
    try {
      const source = await settingsBackend.connectionDetails(connection.id)
      connect.reset()
      test.reset()
      setPrefill(null)
      setDuplicate({
        of: connection.name,
        prefill: duplicatePrefill(choice, source),
      })
      setDriver(choice)
    } catch (error) {
      toast.add({
        title: "Cannot duplicate this connection",
        description: failureOf(error)?.message,
        type: "error",
      })
    }
  }

  const refreshCatalogOf = async (open: OpenConnection) => {
    try {
      const outcome = await metadata.refreshCatalog(
        newCommandId(),
        open.connection,
        open.session,
        null
      )
      if (outcome.type === "denied")
        toast.add({
          title: "Catalog not refreshed",
          description: outcome.reason,
          type: "error",
        })
      else if (outcome.type === "catalogRefreshed")
        toast.add({
          title: `Catalog of ${open.name} refreshed`,
          type: "success",
        })
    } catch (error) {
      toast.add({
        title: "Catalog not refreshed",
        description: failureOf(error)?.message,
        type: "error",
      })
    } finally {
      // The hidden workspace's tree reads it again, as after its own refresh.
      await queryClient.invalidateQueries({
        queryKey: ["catalog", open.connection],
      })
      await queryClient.invalidateQueries({
        queryKey: ["catalog-search", open.connection],
      })
    }
  }

  const browse = async () => {
    const chosen = await openDialog({ multiple: false, directory: false })
    return typeof chosen === "string" ? chosen : null
  }

  return (
    <ConnectionScreenView
      connections={saved.data}
      connectionsError={failureOf(saved.error)}
      onRetryConnections={() => void saved.refetch()}
      drivers={drivers.data}
      driversError={failureOf(drivers.error)}
      onRetryDrivers={() => void drivers.refetch()}
      driver={driver}
      duplicate={duplicate}
      onChooseDriver={(choice) => {
        connect.reset()
        test.reset()
        setDuplicate(null)
        setPrefill(null)
        setDriver(choice)
      }}
      onLeaveDriver={() => {
        connect.reset()
        test.reset()
        setDuplicate(null)
        setPrefill(null)
        setDriver(null)
      }}
      opening={reconnect.isPending ? reconnect.variables.id : null}
      openError={failureOf(reconnect.error)}
      onOpen={(connection) => {
        if (idle()) reconnect.mutate(connection)
      }}
      onRetryOpen={() => {
        if (reconnect.variables) reconnect.mutate(reconnect.variables)
      }}
      openConnectionId={leftOpen?.connection ?? null}
      connectionMenu={connectionMenu}
      submitting={connect.isPending || decide.isPending}
      formError={failureOf(connect.error ?? decide.error)}
      onSubmit={(draft) => {
        if (connect.isPending || test.isPending) return
        connect.mutate(typedSecrets.hold(draft))
      }}
      testing={test.isPending}
      testResult={
        test.data && test.data.type !== "cancelled" ? test.data : null
      }
      testError={failureOf(test.error)}
      onTest={(draft) => {
        if (connect.isPending || test.isPending) return
        test.mutate(typedSecrets.hold(draft))
      }}
      onDraftChange={test.reset}
      onBrowse={browse}
      prefill={prefill}
      cancelling={cancelling}
      onCancelOpening={cancelOpening}
      approval={approval}
      deciding={decide.isPending}
      onDecide={(approved) => {
        if (approval) decide.mutate({ command: approval.command, approved })
      }}
      onReturnToWorkspace={
        leftOpen ? () => void navigate({ to: "/workspace" }) : undefined
      }
      onOpenLocalWork={() => void navigate({ to: "/recovery" })}
    />
  )
}
