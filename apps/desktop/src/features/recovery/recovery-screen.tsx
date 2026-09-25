import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { useHotkeys } from "@tanstack/react-hotkeys"
import { useNavigate, useSearch } from "@tanstack/react-router"
import { useStore } from "@tanstack/react-store"

import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { RecoveryList } from "@/components/oxyn/recovery-list"
import type {
  RecoverableObject,
  RecoveryState,
} from "@/components/oxyn/recovery-list"
import {
  declineObjectPlace,
  markRecoveryOffered,
  restoreWorkingCopies,
  session,
} from "@/features/session"
import {
  savedObjectPlace,
  sectionLabel,
} from "@/features/workspace/object-location"
import { BackendError, newCommandId } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import type { DocumentEntry } from "@/lib/ipc/library"
import { recovery } from "@/lib/ipc/recovery"
import { settingsBackend } from "@/lib/ipc/settings"

function failureOf(error: unknown): BackendFailure {
  return error instanceof BackendError
    ? { message: error.message, retryable: error.retryable }
    : { message: String(error), retryable: false }
}

/** A connection named, or what is known of it: deleted only once observed. */
function connectionLabel(
  connections: { data?: Array<{ id: string; name: string }>; isError: boolean },
  id: string
) {
  if (connections.data)
    return (
      connections.data.find((known) => known.id === id)?.name ??
      "A deleted connection"
    )
  return connections.isError
    ? "Its connection could not be listed"
    : "Reading its connection…"
}

/**
 * The working copies a previous launch left open (ADR-0021).
 *
 * Choosing hands text to the next workspace, which reopens it in consoles once
 * the user has chosen a connection. Nothing connects and nothing runs here.
 */
export function RecoveryScreen() {
  const navigate = useNavigate()
  const { startup = false } = useSearch({ from: "/recovery" })
  const [cursors, setCursors] = React.useState<Array<string | null>>([null])
  const cursor = cursors[cursors.length - 1] ?? null
  const [selected, setSelected] = React.useState(
    () => new Map<string, DocumentEntry>()
  )
  const selectedOnce = React.useRef(false)

  const status = useQuery({
    queryKey: ["recovery-status"],
    queryFn: () => recovery.status(),
    staleTime: Infinity,
  })
  const page = useQuery({
    queryKey: ["recovery-working-copies", cursor],
    queryFn: () =>
      library.listDocuments(newCommandId(), {
        savedOnly: false,
        openOnly: true,
        search: "",
        before: cursor,
        limit: null,
      }),
  })

  // The object tab saved last: read from the workspace, never a server.
  const place = useQuery({
    queryKey: ["recovery-object-place"],
    queryFn: savedObjectPlace,
    staleTime: Infinity,
  })
  const connections = useQuery({
    queryKey: ["connections"],
    queryFn: settingsBackend.listConnections,
    enabled: place.data != null,
  })
  // Chosen by default, as the working copies are after a crash.
  const [objectSelected, setObjectSelected] = React.useState(true)
  const saved = place.data ?? null
  const object: RecoverableObject | null = saved
    ? {
        name: saved.address.relation ?? "",
        section: sectionLabel(saved.section),
        connection: connectionLabel(connections, saved.connection),
      }
    : null

  // After a crash every working copy starts selected.
  React.useEffect(() => {
    if (selectedOnce.current || !page.data) return
    selectedOnce.current = true
    setSelected(new Map(page.data.entries.map((entry) => [entry.id, entry])))
  }, [page.data])

  const state: RecoveryState = page.isPending
    ? { status: "loading" }
    : page.isError
      ? { status: "error", error: failureOf(page.error) }
      : { status: "ready", entries: page.data.entries }

  const leave = (objectRestored: boolean) => {
    // Not reopened this launch unless chosen; saved all the same.
    declineObjectPlace(!objectRestored)
    markRecoveryOffered()
    void navigate({ to: "/" })
  }
  // Back returns where the user came from: the open workspace, if any. Nothing
  // is restored on the way.
  const hasWorkspace = useStore(session, (current) => current.open !== null)
  const back = () => {
    markRecoveryOffered()
    void navigate({ to: hasWorkspace ? "/workspace" : "/" })
  }
  useHotkeys([
    { hotkey: "Escape", callback: back },
    { hotkey: "Mod+[", callback: back },
  ])

  return (
    <main className="flex h-full min-h-0 flex-col bg-background">
      {/* The window has no title bar of its own (`titleBarStyle: Overlay`):
          without a drag region this screen cannot be moved, and the macOS
          traffic lights would sit over its first line. */}
      <div
        data-tauri-drag-region
        aria-hidden
        className="h-12 shrink-0"
        data-slot="recovery-title-bar"
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <RecoveryList
          // The backend's `abnormal` holds for the whole process: only the
          // opening that followed the launch may repeat it.
          abnormal={startup && (status.data?.abnormal ?? false)}
          unresolvedWrite={status.data?.unresolvedWrite ?? false}
          state={state}
          selected={new Set(selected.keys())}
          onToggle={(entry) =>
            setSelected((current) => {
              const next = new Map(current)
              if (!next.delete(entry.id)) next.set(entry.id, entry)
              return next
            })
          }
          hasPrevious={cursors.length > 1}
          hasNext={page.data?.next != null}
          onPrevious={() => setCursors((all) => all.slice(0, -1))}
          onNext={() => {
            const next = page.data?.next
            if (next) setCursors((all) => [...all, next])
          }}
          onRefresh={() => void page.refetch()}
          object={object}
          objectSelected={objectSelected}
          onToggleObject={() => setObjectSelected((chosen) => !chosen)}
          onRestore={() => {
            restoreWorkingCopies([...selected.values()])
            leave(object !== null && objectSelected)
          }}
          onContinue={() => leave(false)}
          backLabel={hasWorkspace ? "Back to workspace" : "Back to connections"}
          onBack={back}
        />
      </div>
    </main>
  )
}
