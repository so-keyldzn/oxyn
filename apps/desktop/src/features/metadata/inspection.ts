import { createStore } from "@tanstack/react-store"

import type { CatalogAddress, ResultColumn } from "@/lib/ipc/types"

/** The row a grid has selected, with what the inspector needs to show it. */
export interface SelectedRow {
  /** The grid it comes from: the SQL console or a relation preview. */
  source: string
  connection: string
  connectionName: string
  result: string
  columns: Array<ResultColumn>
  row: number
  column: number
}

/** A value whose full text is being inspected. */
export interface InspectedValue {
  /** The grid that asked: only the dialog mounted for it opens. */
  source: string
  connection: string
  connectionName: string
  result: string
  column: number
  columnName: string
  row: number
}

export interface InspectionState {
  /** The relation the object view shows, for the object inspector. */
  object: { connection: string; address: CatalogAddress } | null
  selectedRow: SelectedRow | null
  inspected: InspectedValue | null
}

/**
 * What the side column inspects, shared by the views that select and the
 * panels that show. Local state only: nothing here is persisted, and nothing
 * here reaches the backend on its own.
 */
export const inspection = createStore<InspectionState>({
  object: null,
  selectedRow: null,
  inspected: null,
})

export function inspectObject(object: InspectionState["object"]) {
  inspection.setState((state) => ({ ...state, object }))
}

export function selectRow(selectedRow: SelectedRow | null) {
  inspection.setState((state) => ({ ...state, selectedRow }))
}

export function selectInspectorColumn(column: number) {
  inspection.setState((state) =>
    state.selectedRow
      ? { ...state, selectedRow: { ...state.selectedRow, column } }
      : state
  )
}

export function inspectValue(inspected: InspectedValue | null) {
  inspection.setState((state) => ({ ...state, inspected }))
}

/** Forgets a result that is no longer shown, and its selection with it. */
export function releaseResult(result: string) {
  inspection.setState((state) => ({
    ...state,
    selectedRow:
      state.selectedRow?.result === result ? null : state.selectedRow,
    inspected: state.inspected?.result === result ? null : state.inspected,
  }))
}
