import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"

import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { completion } from "@/lib/ipc/completion"
import type { CompletionCatalog } from "@/lib/ipc/completion"

/**
 * A catalog load announces itself once per level it reads: a pass over a
 * schema is a burst, and the snapshot is read once when it quiets down.
 */
const COALESCE_MS = 150

/**
 * The snapshot of `connection`'s catalog cache, for the editor's completion
 * (ADR-0057).
 *
 * It is a cache read — the backend asks no server for it — made when the
 * editor opens and again when a signal says the cache may hold other content:
 * a level was read (`catalogChanged`), a DDL invalidated it
 * (`catalogInvalidated`), or signals were missed (`lagged`). **Never on a
 * keystroke**: the completion reads the copy this returns.
 *
 * `null` until the first read, and while the connection has no cache (not
 * connected yet): the completion then offers its keywords only.
 */
export function useCompletionCatalog(
  connection: string
): CompletionCatalog | null {
  const queryClient = useQueryClient()
  const query = useQuery({
    queryKey: ["completion", connection],
    queryFn: () => completion.catalog(connection),
    // Read again by the signals, not by the clock or by a window regaining
    // the focus.
    staleTime: Infinity,
    retry: false,
  })

  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null)
  useRefreshSignal(connection, (signal) => {
    if (
      signal.type !== "catalogChanged" &&
      signal.type !== "catalogInvalidated" &&
      signal.type !== "lagged"
    )
      return
    if (timer.current !== null) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      timer.current = null
      void queryClient.invalidateQueries({
        queryKey: ["completion", connection],
      })
    }, COALESCE_MS)
  })
  React.useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current)
    },
    []
  )

  return query.data ?? null
}
