import * as React from "react"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  disconnectRequests,
  finishWorkspaceDisconnect,
} from "./disconnect-requests"
import {
  showConnection,
  closeConnection,
  connectionReleased,
  session,
  takeConnectionsLeftByReload,
} from "@/features/session"
import { ExplorerProvider } from "@/features/multi-base/explorer-provider"
import {
  CatchBoundary,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router"
import { useSelector } from "@tanstack/react-store"

import { useAsidePanels } from "@/features/workspace/aside-panels"
import { release } from "@/features/workspace/release-workspace"
import { RouteError } from "@/features/workspace/route-failures"
import { WorkspaceScreen } from "@/features/workspace/workspace-screen"
import type { WorkspaceExit } from "@/features/workspace/workspace-screen"
import { backend, newCommandId } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * The workspaces of the open connections, mounted above the routes so that
 * showing the start screen does not destroy them (« Switch connection »,
 * « Return to workspace · Esc »). The shown one is visible on `/workspace`;
 * every other stays mounted, hidden, with its consoles, results and sessions
 * (ADR-0046).
 *
 * A workspace ends only when the user disconnects it, or when its connection
 * is reopened on new sessions: its drafts are written, then its sessions
 * close. Showing another connection closes nothing.
 */
export function WorkspaceHost({
  onOpenSettings,
}: {
  onOpenSettings?: () => void
}) {
  const navigate = useNavigate()
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const open = useSelector(session, (state) => state.open)
  const workspaces = useSelector(session, (state) => state.workspaces)
  // What is mounted: the store's workspaces, plus those still writing their
  // drafts on their way out.
  const [mounted, setMounted] = React.useState<Array<OpenConnection>>(
    () => workspaces
  )
  const disconnects = useSelector(disconnectRequests)
  React.useEffect(() => {
    const target = disconnects[0]
    if (!target) return
    if (showConnection(target)) void navigate({ to: "/workspace" })
    else finishWorkspaceDisconnect(target, false)
  }, [disconnects, navigate])
  const [releaseErrors, setReleaseErrors] = React.useState<
    Record<string, string>
  >({})
  const [retry, retryRelease] = React.useReducer((n: number) => n + 1, 0)
  const exits = React.useRef(new Map<string, WorkspaceExit>())
  const releasing = React.useRef(new Set<string>())

  React.useEffect(() => {
    if (
      workspaces.some(
        (next) => !mounted.some((held) => held.session === next.session)
      )
    )
      setMounted((current) => [
        ...current,
        ...workspaces.filter(
          (next) => !current.some((held) => held.session === next.session)
        ),
      ])
    for (const previous of mounted) {
      if (workspaces.some((held) => held.session === previous.session)) continue
      if (releasing.current.has(previous.session)) continue
      releasing.current.add(previous.session)
      void (async () => {
        try {
          await release(
            previous,
            () =>
              session.state.workspaces.some(
                (held) => held.connection === previous.connection
              ),
            exits.current.get(previous.session) ?? null
          )
          releasing.current.delete(previous.session)
          connectionReleased(previous.connection)
          finishWorkspaceDisconnect(previous.connection, true)
          setReleaseErrors((all) => {
            const next = { ...all }
            delete next[previous.session]
            return next
          })
          setMounted((current) =>
            current.filter((held) => held.session !== previous.session)
          )
        } catch (caught) {
          finishWorkspaceDisconnect(previous.connection, false)
          setReleaseErrors((all) => ({
            ...all,
            [previous.session]:
              caught instanceof Error ? caught.message : String(caught),
          }))
        }
      })()
    }
  }, [workspaces, mounted, retry])

  // A reload of the webview keeps the Rust sessions but loses what they
  // belonged to: close them, then offer the working copies they left. Nothing
  // claims a crash — this was a reload (ADR-0021).
  React.useEffect(() => {
    const connections = takeConnectionsLeftByReload()
    if (connections.length === 0) return
    void (async () => {
      await Promise.allSettled(
        connections.map((connection) => backend.disconnect(connection))
      )
      const left = await library
        .listDocuments(newCommandId(), {
          savedOnly: false,
          openOnly: true,
          search: "",
          before: null,
          limit: 1,
        })
        .catch(() => null)
      if (left && left.entries.length > 0) await navigate({ to: "/recovery" })
    })()
  }, [navigate])

  return (
    <ExplorerProvider>
      {Object.entries(releaseErrors).map(([id, message]) => (
        <Alert key={id} className="fixed bottom-4 left-4 z-50 max-w-md">
          <AlertTitle>
            Workspace disconnect incomplete ·{" "}
            {mounted.find((held) => held.session === id)?.name}
          </AlertTitle>
          <AlertDescription>{message}</AlertDescription>
          <Button
            variant="outline"
            onClick={() => {
              releasing.current.delete(id)
              retryRelease()
            }}
          >
            Retry cleanup
          </Button>
        </Alert>
      ))}
      {mounted.map((held) => {
        // The same session with new settings (a tier changed) is the live
        // value; a workspace on its way out keeps the one it had.
        const current =
          workspaces.find((live) => live.session === held.session) ?? null
        const visible =
          pathname === "/workspace" &&
          current !== null &&
          open?.session === held.session
        return (
          <RetainedWorkspace
            key={held.session}
            open={current ?? held}
            visible={visible}
            onOpenSettings={onOpenSettings}
            exitRef={(exit) => {
              if (exit) exits.current.set(held.session, exit)
              else exits.current.delete(held.session)
            }}
            onSwitchConnection={() => void navigate({ to: "/" })}
            onDisconnect={() => {
              closeConnection(held.connection)
              if (session.state.open) void navigate({ to: "/workspace" })
              else void navigate({ to: "/" })
            }}
          />
        )
      })}
    </ExplorerProvider>
  )
}

/**
 * One workspace of the host, with the right-column panels of **its**
 * connection: an assistant composed for the shown connection would answer,
 * behind a hidden workspace, under another connection's tier (I-04).
 */
function RetainedWorkspace({
  open,
  visible,
  onOpenSettings,
  exitRef,
  onSwitchConnection,
  onDisconnect,
}: {
  open: OpenConnection
  visible: boolean
  onOpenSettings?: () => void
  exitRef: React.Ref<WorkspaceExit>
  onSwitchConnection: () => void
  onDisconnect: () => void
}) {
  const aside = useAsidePanels(open)
  return (
    <div
      data-slot="workspace-host"
      data-connection={open.connection}
      className={visible ? "h-full" : "hidden"}
      // Hidden, it keeps its state but leaves the accessibility tree.
      inert={!visible}
    >
      {/* Rendered outside the routes: their error screens do not reach
          here. */}
      <CatchBoundary
        getResetKey={() => open.session}
        errorComponent={RouteError}
      >
        <WorkspaceScreen
          open={open}
          visible={visible}
          aside={aside}
          onOpenSettings={onOpenSettings}
          exitRef={exitRef}
          onSwitchConnection={onSwitchConnection}
          onDisconnect={onDisconnect}
        />
      </CatchBoundary>
    </div>
  )
}
