import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import { AssistantToolRows } from "@/components/oxyn/assistant-tool-rows"
import type { ToolRowsState } from "@/components/oxyn/assistant-tool-rows"
import type { ToolCallEntry } from "@/features/assistant/transcript"
import { openAgentResult } from "@/features/workspace/result-requests"
import { BackendError } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import { results } from "@/lib/ipc/results"

/**
 * The rows of an agent's query, read from the buffer the executor retained.
 *
 * Opening goes through `OpenRetainedResult`, which checks the result belongs
 * to this connection; pages through `read_result_page`, the console's own
 * call. Neither reaches a server, and neither tells a model anything.
 */
export function ToolRows({
  connection,
  entry,
}: {
  connection: string
  entry: ToolCallEntry & { result: string }
}) {
  const opened = useQuery({
    queryKey: ["assistant-tool-rows", connection, entry.result],
    queryFn: () => library.openRetainedResult(connection, entry.result),
    // Whether rows are still held is an answer: refetching on focus would
    // turn a grid being read into « no longer available ».
    staleTime: Infinity,
  })

  const fetchPage = React.useCallback(
    (offset: number, limit: number) =>
      results.readResultPage(connection, entry.result, offset, limit),
    [connection, entry.result]
  )

  const state: ToolRowsState = opened.isPending
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
            truncated: opened.data.truncated,
          }

  return (
    <AssistantToolRows
      state={state}
      fetchPage={fetchPage}
      onOpenAll={() =>
        openAgentResult(connection, entry.result, entry.statement)
      }
      onRetry={() => void opened.refetch()}
    />
  )
}
