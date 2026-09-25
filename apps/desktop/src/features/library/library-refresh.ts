import { useQueryClient } from "@tanstack/react-query"
import type { QueryClient } from "@tanstack/react-query"

import { useRefreshSignal } from "@/features/metadata/refresh-signals"

/**
 * Every library query starts with this key. A run written to the history, a
 * document written, a missed signal, or the Refresh button invalidates it.
 */
export const LIBRARY_QUERY_KEY = ["library"] as const

/**
 * Marks the library stale. TanStack reads again only the queries in use, so
 * an open library refreshes and a closed one reads on its next opening
 * (ADR-0022).
 */
export function refreshLibrary(queryClient: QueryClient) {
  return queryClient.invalidateQueries({ queryKey: LIBRARY_QUERY_KEY })
}

/**
 * Reads the library again when a run, the user's or an agent's, reaches the
 * history. Runs of every connection count: the history spans them.
 */
export function useLibraryRefresh() {
  const queryClient = useQueryClient()
  useRefreshSignal(null, (signal) => {
    if (signal.type !== "historyRecorded" && signal.type !== "lagged") return
    void refreshLibrary(queryClient)
  })
}
