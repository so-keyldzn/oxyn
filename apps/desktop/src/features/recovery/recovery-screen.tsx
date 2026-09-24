import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { useHotkeys } from "@tanstack/react-hotkeys"
import { useNavigate, useSearch } from "@tanstack/react-router"
import { useStore } from "@tanstack/react-store"

import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { RecoveryList } from "@/components/oxyn/recovery-list"
import type { RecoveryState } from "@/components/oxyn/recovery-list"
import {
  markRecoveryOffered,
  restoreWorkingCopies,
  session,
} from "@/features/session"
import { BackendError, newCommandId } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import type { DocumentEntry } from "@/lib/ipc/library"
import { recovery } from "@/lib/ipc/recovery"

function failureOf(error: unknown): BackendFailure {
  return error instanceof BackendError
    ? { message: error.message, retryable: error.retryable }
    : { message: String(error), retryable: false }
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

  const leave = () => {
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
          onRestore={() => {
            restoreWorkingCopies([...selected.values()])
            leave()
          }}
          onContinue={leave}
          backLabel={hasWorkspace ? "Back to workspace" : "Back to connections"}
          onBack={back}
        />
      </div>
    </main>
  )
}
