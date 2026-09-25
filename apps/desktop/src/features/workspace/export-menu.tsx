import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import { ExportMenuView } from "@/components/oxyn/export-menu"
import type { ExportState } from "@/components/oxyn/export-menu"
import { toast } from "@/components/ui/toast"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { results } from "@/lib/ipc/results"
import type { ExportFormatChoice } from "@/lib/ipc/results"

/**
 * Export of a result the user sees.
 *
 * Offered on a **whole** result only: a truncated or cancelled buffer looks
 * finished and is not. The save dialog is opened by the backend, which writes
 * where the user chose: no path ever crosses the webview (SECURITY, « Surface
 * d'entrée »).
 */
export function ExportMenu({
  connection,
  result,
  exportable,
  reason,
  label,
  scope,
  defaultName = "result",
}: {
  connection: string
  result: string | null
  exportable: boolean
  reason: string
  /** `Export preview…` on a relation preview. */
  label?: string
  /** What the file holds, said above the formats. */
  scope?: string
  /** A file name suggestion; the backend keeps only a name from it. */
  defaultName?: string
}) {
  const exporter = useResultExport({ connection, result, defaultName })
  return (
    <ExportMenuView
      formats={exporter.formats}
      formatsFailed={exporter.formatsFailed}
      exportable={exportable && result !== null}
      reason={reason}
      state={exporter.state}
      label={label}
      scope={scope}
      onExport={exporter.run}
      onCancel={exporter.cancel}
    />
  )
}

/**
 * The export of one result, apart from where it is offered: a footer button
 * at wide width, a submenu of `Actions` below 1200 px. Held by the view, not
 * by a menu: a menu unmounts when it closes, and leaving cancels the write.
 */
export function useResultExport({
  connection,
  result,
  defaultName = "result",
}: {
  connection: string
  result: string | null
  defaultName?: string
}) {
  const [state, setState] = React.useState<ExportState>({ status: "idle" })
  const running = React.useRef<string | null>(null)
  const formats = useQuery({
    queryKey: ["export-formats"],
    queryFn: results.exportFormats,
    staleTime: Number.POSITIVE_INFINITY,
  })

  // An export outlives nothing: leaving the view cancels the write.
  React.useEffect(
    () => () => {
      if (running.current) void backend.cancel(running.current)
    },
    []
  )

  const run = async (format: ExportFormatChoice) => {
    // A second click while the dialog is open or a write runs does nothing;
    // the ref, not the state, because both clicks may share one render.
    if (!result || !format.supported || running.current) return
    const id = newCommandId()
    running.current = id
    // The native dialog is modal: while it is open nothing here is reachable,
    // and a Cancel pressed before the write starts has nothing to stop.
    setState({ status: "exporting", format: format.label, cancelling: false })
    try {
      const outcome = await results.exportResult(
        id,
        connection,
        result,
        format.format,
        defaultName
      )
      if (outcome === null) return
      if (outcome.type === "exported") {
        toast.add({
          title: "Export complete",
          description:
            outcome.rows === 0
              ? "No rows written: the result was empty."
              : `${outcome.rows.toLocaleString("en-US")} rows written · ${outcome.bytes.toLocaleString("en-US")} bytes.`,
          type: "success",
        })
      } else if (outcome.type === "denied") {
        toast.add({
          title: "Export refused",
          description: outcome.reason,
          type: "error",
        })
      } else if (outcome.type === "cancelled") {
        toast.add({
          title: "Export cancelled",
          description: "The file may be partially written.",
          type: "warning",
        })
      }
    } catch (error) {
      toast.add({
        title: "Export failed",
        description:
          error instanceof BackendError
            ? `${error.message} ${error.retryable ? "The export can be tried again as is." : "Trying the same export again gives the same result."}`
            : String(error),
        type: "error",
      })
    } finally {
      running.current = null
      setState({ status: "idle" })
    }
  }

  return {
    formats: formats.data ?? null,
    formatsFailed: formats.isError,
    state,
    run: (format: ExportFormatChoice) => void run(format),
    cancel: () => {
      if (!running.current || state.status !== "exporting") return
      setState({ ...state, cancelling: true })
      void backend.cancel(running.current)
    },
  }
}
