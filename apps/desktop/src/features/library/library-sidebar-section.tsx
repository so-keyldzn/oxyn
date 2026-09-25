import * as React from "react"
import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"
import { useDebouncedValue } from "@tanstack/react-pacer"

import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { LibraryEntryView } from "@/components/oxyn/library-entry-view"
import type { LibraryEntryState } from "@/components/oxyn/library-entry-view"
import { LibraryPanel } from "@/components/oxyn/library-panel"
import type {
  ConnectionOption,
  HistoryFilters,
  LibraryState,
  LibraryView,
} from "@/components/oxyn/library-panel"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { settingsBackend } from "@/lib/ipc/settings"
import { library } from "@/lib/ipc/library"
import type {
  DocumentEntry,
  DocumentView,
  HistoryDetail,
  HistoryRow,
} from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

/** Every library query starts with this key: a run invalidates it. */
export const LIBRARY_QUERY_KEY = ["library"] as const

/** `retryable` comes from the backend; anything else is not worth retrying. */
function backendFailure(error: unknown): BackendFailure {
  return error instanceof BackendError
    ? { message: error.message, retryable: error.retryable }
    : { message: String(error), retryable: false }
}

/**
 * The library section of the left sidebar: history and saved queries.
 *
 * Opening something hands text to the workspace, which opens a new console
 * with its own session. Nothing here runs anything.
 */
export function LibrarySidebarSection({
  open,
  onOpenHistory,
  onOpenResult,
  onOpenCopy,
  onResume,
}: {
  open: OpenConnection
  onOpenHistory: (entry: HistoryDetail) => void
  onOpenResult: (row: HistoryRow) => void
  onOpenCopy: (document: DocumentView, entry: DocumentEntry) => void
  onResume: (document: DocumentView) => void
}) {
  const queryClient = useQueryClient()
  const [view, setView] = React.useState<LibraryView>("history")
  const [search, setSearch] = React.useState("")
  const [debouncedSearch] = useDebouncedValue(search, { wait: 250 })
  const [filters, setFilters] = React.useState<HistoryFilters>({
    connection: "",
    days: 7,
    status: "all",
  })
  // Cursors of the pages before the one shown; the last is the current one.
  const [cursors, setCursors] = React.useState<Array<string | number | null>>([
    null,
  ])
  const [actionError, setActionError] = React.useState<BackendFailure | null>(
    null
  )
  // The read the user cancelled stays cancelled until they ask again: an
  // empty list there would claim that nothing matched (UX-SPEC l.410-411).
  const [cancelled, setCancelled] = React.useState(false)
  // The command of the read in flight, which `backend.cancel` stops in SQLite.
  const reading = React.useRef<string | null>(null)
  const cursor = cursors[cursors.length - 1] ?? null

  const resetPages = () => {
    setCursors([null])
    setCancelled(false)
  }

  const track = async <T,>(read: (commandId: string) => Promise<T>) => {
    const id = newCommandId()
    reading.current = id
    try {
      return await read(id)
    } finally {
      if (reading.current === id) reading.current = null
    }
  }

  // Recent results is the history filtered on a result reference: same
  // filters, same pages, same rows.
  const resultsOnly = view === "results"
  const historyKey = [
    ...LIBRARY_QUERY_KEY,
    "history",
    resultsOnly,
    debouncedSearch,
    filters,
    cursor,
  ]
  const history = useQuery({
    queryKey: historyKey,
    queryFn: () =>
      track((id) =>
        library.readHistory(id, {
          connection: filters.connection === "" ? null : filters.connection,
          search: debouncedSearch,
          days: filters.days,
          status: filters.status,
          before: typeof cursor === "number" ? cursor : null,
          limit: null,
          resultsOnly,
        })
      ),
    enabled: view !== "saved",
    placeholderData: keepPreviousData,
  })

  const savedKey = [...LIBRARY_QUERY_KEY, "saved", debouncedSearch, cursor]
  const saved = useQuery({
    queryKey: savedKey,
    queryFn: () =>
      track((id) =>
        library.listDocuments(id, {
          savedOnly: true,
          openOnly: false,
          search: debouncedSearch,
          before: typeof cursor === "string" ? cursor : null,
          limit: null,
        })
      ),
    enabled: view === "saved",
    placeholderData: keepPreviousData,
  })

  const savedConnections = useQuery({
    queryKey: ["connections"],
    queryFn: () => settingsBackend.listConnections(),
  })
  const recorded = useQuery({
    queryKey: [...LIBRARY_QUERY_KEY, "history-connections"],
    queryFn: () => library.historyConnections(null),
  })

  const connections = React.useMemo<Array<ConnectionOption>>(() => {
    const options: Array<ConnectionOption> = (savedConnections.data ?? []).map(
      (connection) => ({ value: connection.id, label: connection.name })
    )
    for (const entry of recorded.data?.entries ?? []) {
      if (options.some((option) => option.value === entry.connection)) continue
      const name = entry.name ?? "Unnamed connection"
      options.push({
        value: entry.connection,
        label: entry.inWorkspace ? name : `${name} · not in this workspace`,
      })
    }
    return options
  }, [savedConnections.data, recorded.data])

  // The entry whose full text is shown read only, beside the console.
  const [inspecting, setInspecting] = React.useState<
    | { kind: "history"; row: HistoryRow }
    | { kind: "saved"; entry: DocumentEntry }
    | null
  >(null)
  const inspected = useQuery({
    queryKey: [
      ...LIBRARY_QUERY_KEY,
      "entry",
      inspecting?.kind,
      inspecting?.kind === "history" ? inspecting.row.id : inspecting?.entry.id,
    ],
    queryFn: async (): Promise<LibraryEntryState> => {
      if (inspecting?.kind === "history")
        return {
          status: "history",
          entry: await library.readHistoryEntry(inspecting.row.id),
        }
      if (inspecting?.kind === "saved")
        return {
          status: "saved",
          document: await library.openDocument(inspecting.entry.id),
          entry: inspecting.entry,
        }
      return { status: "loading" }
    },
    enabled: inspecting !== null,
  })
  const entryState: LibraryEntryState = inspected.isError
    ? { status: "error", error: backendFailure(inspected.error) }
    : (inspected.data ?? { status: "loading" })

  const current = view === "saved" ? saved : history
  const state: LibraryState = actionError
    ? { status: "error", error: actionError }
    : cancelled
      ? { status: "cancelled" }
      : current.isPending
        ? { status: "loading" }
        : current.isError
          ? { status: "error", error: backendFailure(current.error) }
          : view === "saved"
            ? { status: "saved", entries: saved.data?.entries ?? [] }
            : { status: "history", entries: history.data?.entries ?? [] }
  const next = view === "saved" ? saved.data?.next : history.data?.next

  const cancel = () => {
    const id = reading.current
    if (id) void backend.cancel(id).catch(() => undefined)
    setCancelled(true)
    // Drops the answer too: a read that finished anyway does not replace
    // « cancelled » with a list.
    void queryClient.cancelQueries({
      queryKey: view === "saved" ? savedKey : historyKey,
    })
  }

  const guard = async (action: () => Promise<void>) => {
    setActionError(null)
    try {
      await action()
    } catch (error) {
      setActionError(backendFailure(error))
    }
  }

  return (
    <>
      <LibraryEntryView
        open={inspecting !== null}
        state={entryState}
        driver={open.driver}
        destination={open.name}
        onRetry={() => void inspected.refetch()}
        onClose={() => setInspecting(null)}
        onOpenCopy={() => {
          const data = inspected.data
          setInspecting(null)
          if (data?.status === "history") onOpenHistory(data.entry)
          else if (data?.status === "saved")
            onOpenCopy(data.document, data.entry)
        }}
      />
      <LibraryPanel
        view={view}
        onViewChange={(nextView) => {
          setView(nextView)
          resetPages()
        }}
        search={search}
        onSearchChange={(value) => {
          setSearch(value)
          resetPages()
        }}
        filters={filters}
        onFiltersChange={(value) => {
          setFilters(value)
          resetPages()
        }}
        connections={connections}
        state={state}
        hasPrevious={cursors.length > 1}
        hasNext={next !== null && next !== undefined}
        onPrevious={() => {
          setCancelled(false)
          setCursors((all) => all.slice(0, -1))
        }}
        onNext={() => {
          setCancelled(false)
          if (next !== null && next !== undefined)
            setCursors((all) => [...all, next])
        }}
        searching={current.isFetching && !cancelled}
        onCancel={cancel}
        onRefresh={() => {
          setActionError(null)
          setCancelled(false)
          void queryClient.invalidateQueries({ queryKey: LIBRARY_QUERY_KEY })
        }}
        currentConnection={open.connection}
        currentConnectionName={open.name}
        onInspectHistory={(row) => setInspecting({ kind: "history", row })}
        onInspectSaved={(entry) => setInspecting({ kind: "saved", entry })}
        onOpenResult={onOpenResult}
        onOpenHistory={(row) =>
          void guard(async () =>
            onOpenHistory(await library.readHistoryEntry(row.id))
          )
        }
        onReconcile={(row) =>
          void guard(async () => {
            await library.reconcileHistoryEntry(row.id)
            await queryClient.invalidateQueries({ queryKey: LIBRARY_QUERY_KEY })
          })
        }
        onOpenSaved={(entry) =>
          void guard(async () =>
            onOpenCopy(await library.openDocument(entry.id), entry)
          )
        }
        onResumeSaved={(entry) =>
          void guard(async () => onResume(await library.openDocument(entry.id)))
        }
        onDeleteSaved={(entry) =>
          void guard(async () => {
            const document = await library.openDocument(entry.id)
            await library.deleteDocument(
              entry.id,
              Math.max(document.revision, document.savedRevision) + 1
            )
            await queryClient.invalidateQueries({ queryKey: LIBRARY_QUERY_KEY })
          })
        }
      />
    </>
  )
}
