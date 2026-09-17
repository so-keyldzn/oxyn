import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { ValuePageDialog } from "@/components/oxyn/value-page-dialog"
import type { ValuePageState } from "@/components/oxyn/value-page-dialog"
import type { GridPosition } from "@/components/oxyn/grid-selection"
import { copyToClipboard } from "@/features/metadata/clipboard"
import {
  inspectValue,
  inspection,
  selectRow,
} from "@/features/metadata/inspection"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { results } from "@/lib/ipc/results"
import type { OpenConnection, ResultColumn } from "@/lib/ipc/types"

/**
 * The props that connect a result grid to the inspectors: selecting a cell
 * feeds the record inspector, Enter or a double click opens the full value.
 *
 * `source` names the grid, so that the dialog mounted for it — and only that
 * one — opens.
 */
export function gridInspection({
  source,
  open,
  result,
  columns,
}: {
  source: string
  open: OpenConnection
  result: string | null
  columns: Array<ResultColumn>
}) {
  return {
    onActiveChange: (active: GridPosition | null) => {
      if (!result || !active) return
      selectRow({
        source,
        connection: open.connection,
        connectionName: open.name,
        result,
        columns,
        row: active.row,
        column: active.column,
      })
    },
    onInspect: (position: GridPosition) => {
      if (!result) return
      inspectValue({
        source,
        connection: open.connection,
        connectionName: open.name,
        result,
        column: position.column,
        columnName: columns[position.column]?.name ?? "",
        row: position.row,
      })
    },
  }
}

/**
 * The full-value dialog of one grid, driven by `InspectResultValue`.
 *
 * Mount one per grid `source`. Closing while a page loads cancels it; a late
 * answer for a value the user left is dropped.
 */
export function ValueInspectionDialog({ source }: { source: string }) {
  const inspected = useStore(inspection, (state) =>
    state.inspected?.source === source ? state.inspected : null
  )
  const [offsets, setOffsets] = React.useState<Array<number>>([0])
  const [state, setState] = React.useState<ValuePageState>({
    status: "loading",
  })
  const running = React.useRef<string | null>(null)

  const load = React.useCallback(
    async (offset: number) => {
      if (!inspected) return
      if (running.current) void backend.cancel(running.current)
      const id = newCommandId()
      running.current = id
      setState({ status: "loading" })
      try {
        const page = await results.inspectValue(
          id,
          inspected.connection,
          inspected.result,
          inspected.row,
          inspected.column,
          offset
        )
        if (running.current !== id) return
        setState(page ? { status: "page", page } : { status: "expired" })
      } catch (error) {
        if (running.current !== id) return
        setState({
          status: "error",
          message:
            error instanceof BackendError ? error.message : String(error),
        })
      } finally {
        if (running.current === id) running.current = null
      }
    },
    [inspected]
  )

  React.useEffect(() => {
    if (!inspected) return
    setOffsets([0])
    void load(0)
  }, [inspected, load])

  const close = () => {
    if (running.current) void backend.cancel(running.current)
    running.current = null
    inspectValue(null)
  }

  if (!inspected) return null
  const current = offsets[offsets.length - 1] ?? 0

  return (
    <ValuePageDialog
      open
      connectionName={inspected.connectionName}
      column={inspected.columnName}
      row={inspected.row}
      state={state}
      canGoBack={offsets.length > 1}
      onPrevious={() => {
        const previous = offsets.slice(0, -1)
        setOffsets(previous.length > 0 ? previous : [0])
        void load(previous[previous.length - 1] ?? 0)
      }}
      onNext={() => {
        if (state.status !== "page" || state.page.nextOffset === null) return
        const next = state.page.nextOffset
        setOffsets((stack) => [...stack, next])
        void load(next)
      }}
      onRetry={() => void load(current)}
      onCopyPage={(text) => void copyToClipboard(text, "Value")}
      onClose={close}
    />
  )
}

/** Clears the grid selection a view published, when that view goes away. */
export function useReleaseSelection(source: string) {
  React.useEffect(
    () => () => {
      if (inspection.state.selectedRow?.source === source) selectRow(null)
      if (inspection.state.inspected?.source === source) inspectValue(null)
    },
    [source]
  )
}
