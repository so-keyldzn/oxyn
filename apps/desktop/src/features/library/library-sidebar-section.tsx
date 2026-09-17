import * as React from "react"
import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"
import { useDebouncedValue } from "@tanstack/react-pacer"

import { LibraryPanel } from "@/components/oxyn/library-panel"
import type {
  ConnectionOption,
  HistoryFilters,
  LibraryState,
  LibraryView,
} from "@/components/oxyn/library-panel"
import { BackendError, newCommandId } from "@/lib/ipc/client"
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

function failure(error: unknown) {
  return error instanceof BackendError ? error.message : String(error)
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
  const [actionError, setActionError] = React.useState<string | null>(null)
  const cursor = cursors[cursors.length - 1] ?? null

  const resetPages = () => setCursors([null])

  const history = useQuery({
    queryKey: [
      ...LIBRARY_QUERY_KEY,
      "history",
      debouncedSearch,
      filters,
      cursor,
    ],
    queryFn: () =>
      library.readHistory(newCommandId(), {
        connection: filters.connection === "" ? null : filters.connection,
        search: debouncedSearch,
        days: filters.days,
        status: filters.status,
        before: typeof cursor === "number" ? cursor : null,
        limit: null,
      }),
    enabled: view === "history",
    placeholderData: keepPreviousData,
  })

  const saved = useQuery({
    queryKey: [...LIBRARY_QUERY_KEY, "saved", debouncedSearch, cursor],
    queryFn: () =>
      library.listDocuments(newCommandId(), {
        savedOnly: true,
        openOnly: false,
        search: debouncedSearch,
        before: typeof cursor === "string" ? cursor : null,
        limit: null,
      }),
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

  const current = view === "history" ? history : saved
  const state: LibraryState = actionError
    ? { status: "error", message: actionError }
    : current.isPending
      ? { status: "loading" }
      : current.isError
        ? { status: "error", message: failure(current.error) }
        : view === "history"
          ? { status: "history", entries: history.data?.entries ?? [] }
          : { status: "saved", entries: saved.data?.entries ?? [] }
  const next = view === "history" ? history.data?.next : saved.data?.next

  const guard = async (action: () => Promise<void>) => {
    setActionError(null)
    try {
      await action()
    } catch (error) {
      setActionError(failure(error))
    }
  }

  return (
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
      onPrevious={() => setCursors((all) => all.slice(0, -1))}
      onNext={() => {
        if (next !== null && next !== undefined)
          setCursors((all) => [...all, next])
      }}
      onRefresh={() => {
        setActionError(null)
        void queryClient.invalidateQueries({ queryKey: LIBRARY_QUERY_KEY })
      }}
      currentConnection={open.connection}
      onOpenResult={onOpenResult}
      onOpenHistory={(row) =>
        void guard(async () =>
          onOpenHistory(await library.readHistoryEntry(row.id))
        )
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
  )
}
