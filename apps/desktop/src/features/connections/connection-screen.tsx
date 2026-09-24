import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useStore } from "@tanstack/react-store"
import { open as openDialog } from "@tauri-apps/plugin-dialog"

import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { ConnectionScreenView } from "@/features/connections/connection-screen-view"
import type { PendingConnectionApproval } from "@/features/connections/connection-screen-view"
import { useTypedSecrets } from "@/features/connections/typed-secrets"
import type { WithoutSecrets } from "@/features/connections/typed-secrets"
import { openConnection, session } from "@/features/session"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
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
      onChooseDriver={(choice) => {
        connect.reset()
        setDriver(choice)
      }}
      onLeaveDriver={() => {
        connect.reset()
        setDriver(null)
      }}
      opening={reconnect.isPending ? reconnect.variables.id : null}
      openError={failureOf(reconnect.error)}
      onOpen={(connection) => {
        if (reconnect.isPending || connect.isPending) return
        reconnect.mutate(connection)
      }}
      onRetryOpen={() => {
        if (reconnect.variables) reconnect.mutate(reconnect.variables)
      }}
      submitting={connect.isPending || decide.isPending}
      formError={failureOf(connect.error ?? decide.error)}
      onSubmit={(draft) => {
        if (connect.isPending) return
        connect.mutate(typedSecrets.hold(draft))
      }}
      onBrowse={browse}
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
