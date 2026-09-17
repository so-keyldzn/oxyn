import * as React from "react"
import {
  CatchBoundary,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router"
import { useStore } from "@tanstack/react-store"

import {
  closeConnection,
  session,
  takeConnectionLeftByReload,
} from "@/features/session"
import { release } from "@/features/workspace/release-workspace"
import { RouteError } from "@/features/workspace/route-failures"
import { WorkspaceScreen } from "@/features/workspace/workspace-screen"
import type {
  AsideItem,
  WorkspaceExit,
} from "@/features/workspace/workspace-screen"
import { backend, newCommandId } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"

/**
 * The workspace of the open connection, mounted above the routes so that
 * showing the start screen does not destroy it (« Switch connection »,
 * « Return to workspace · Esc »). Shown on `/workspace`, hidden elsewhere.
 *
 * A workspace ends only when another connection replaces it or the user
 * disconnects: its drafts are written, the active text follows to the next
 * connection, then its sessions close. Until then it stays mounted, hidden.
 */
export function WorkspaceHost({
  aside,
  onOpenSettings,
}: {
  aside?: Array<AsideItem>
  onOpenSettings?: () => void
}) {
  const navigate = useNavigate()
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const open = useStore(session, (state) => state.open)
  const [shown, setShown] = React.useState(open)
  const exit = React.useRef<WorkspaceExit>(null)
  const releasing = React.useRef(false)

  React.useEffect(() => {
    if (open?.session === shown?.session || releasing.current) return
    releasing.current = true
    const previous = shown
    void (async () => {
      if (previous) await release(previous, open, exit.current)
      releasing.current = false
      // The next render compares again: a connection opened meanwhile is
      // picked up then.
      setShown(session.state.open)
    })()
  }, [open, shown])

  // A reload of the webview keeps the Rust sessions but loses what they
  // belonged to: close them, then offer the working copies they left. Nothing
  // claims a crash — this was a reload (ADR-0021).
  React.useEffect(() => {
    const connection = takeConnectionLeftByReload()
    if (!connection) return
    void (async () => {
      await backend.disconnect(connection).catch(() => undefined)
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

  if (!shown) return null
  const current = open?.session === shown.session ? open : null
  const visible = pathname === "/workspace" && current !== null
  return (
    <div
      data-slot="workspace-host"
      className={visible ? "h-full" : "hidden"}
      // Hidden, it keeps its state but leaves the accessibility tree.
      inert={!visible}
    >
      {/* Rendered outside the routes: their error screens do not reach here. */}
      <CatchBoundary
        getResetKey={() => shown.session}
        errorComponent={RouteError}
      >
        <WorkspaceScreen
          key={shown.session}
          // The same session with new settings (a tier changed) is the live
          // value; a workspace on its way out keeps the one it had.
          open={current ?? shown}
          visible={visible}
          aside={aside}
          onOpenSettings={onOpenSettings}
          exitRef={exit}
          onSwitchConnection={() => void navigate({ to: "/" })}
          onDisconnect={() => {
            closeConnection()
            void navigate({ to: "/" })
          }}
        />
      </CatchBoundary>
    </div>
  )
}
