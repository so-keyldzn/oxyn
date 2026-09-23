import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import { RetainedResultView } from "@/components/oxyn/retained-result-view"
import type { RetainedResultState } from "@/components/oxyn/retained-result-view"
import { BackendError } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import { results } from "@/lib/ipc/results"

/**
 * A retained result in a workspace tab — a history run's, or an agent query's.
 * Reading only: the statement is never rerun, and an expired result stays
 * expired.
 */
export function RetainedResultTab({
  connection,
  result,
  succeeded,
  onOpenCopy,
}: {
  /** The workspace's connection, which the result must belong to. */
  connection: string
  result: string
  /** The run ended without doubt; otherwise the rows are for inspection. */
  succeeded: boolean
  /** Absent when the statement cannot be opened with its provenance. */
  onOpenCopy?: () => void
}) {
  const opened = useQuery({
    queryKey: ["retained-result", connection, result],
    queryFn: () => library.openRetainedResult(connection, result),
    // Asked once per tab: whether rows are still retained is an answer, and a
    // refetch on focus would silently turn an open tab into « expired ».
    staleTime: Infinity,
    gcTime: 0,
  })

  const resultId = opened.data?.type === "open" ? opened.data.result : null
  const fetchPage = React.useCallback(
    (offset: number, limit: number) =>
      resultId
        ? results.readResultPage(connection, resultId, offset, limit)
        : Promise.reject(new Error("No result")),
    [connection, resultId]
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
            complete: opened.data.complete && succeeded,
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
