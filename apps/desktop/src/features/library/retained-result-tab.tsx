import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import { RetainedResultView } from "@/components/oxyn/retained-result-view"
import type { RetainedResultState } from "@/components/oxyn/retained-result-view"
import { BackendError } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import { results } from "@/lib/ipc/results"
import type { HistoryRow } from "@/lib/ipc/library"

/**
 * A history run's retained rows, in a workspace tab. Reading only: the
 * statement is never rerun, and an expired result stays expired.
 */
export function RetainedResultTab({
  row,
  onOpenCopy,
}: {
  /** A row of this workspace's connection that names a result. */
  row: HistoryRow & { connection: string; result: string }
  onOpenCopy: () => void
}) {
  const opened = useQuery({
    queryKey: ["retained-result", row.connection, row.result],
    queryFn: () => library.openRetainedResult(row.connection, row.result),
    // Asked once per tab: whether rows are still retained is an answer, and a
    // refetch on focus would silently turn an open tab into « expired ».
    staleTime: Infinity,
    gcTime: 0,
  })

  const resultId = opened.data?.type === "open" ? opened.data.result : null
  const fetchPage = React.useCallback(
    (offset: number, limit: number) =>
      resultId
        ? results.readResultPage(row.connection, resultId, offset, limit)
        : Promise.reject(new Error("No result")),
    [row.connection, resultId]
  )

  const state: RetainedResultState = opened.isPending
    ? { status: "loading" }
    : opened.isError
      ? opened.error instanceof BackendError
        ? {
            status: "error",
            message: opened.error.message,
            retryable: opened.error.retryable,
          }
        : { status: "error", message: String(opened.error), retryable: false }
      : opened.data.type === "expired"
        ? { status: "expired" }
        : {
            status: "open",
            result: opened.data.result,
            columns: opened.data.columns,
            rows: opened.data.rows,
            complete: opened.data.complete && row.status === "succeeded",
            truncated: opened.data.truncated,
          }

  return (
    <RetainedResultView
      state={state}
      fetchPage={fetchPage}
      onRetry={() => void opened.refetch()}
      onOpenCopy={onOpenCopy}
    />
  )
}
