import { useQuery } from "@tanstack/react-query"
import { useStore } from "@tanstack/react-store"

import { ObjectInspector } from "@/components/oxyn/object-inspector"
import { ValueInspector } from "@/components/oxyn/value-inspector"
import { copyToClipboard } from "@/features/metadata/clipboard"
import {
  inspectValue,
  inspection,
  selectInspectorColumn,
} from "@/features/metadata/inspection"
import { facetsKey } from "@/features/metadata/use-relation-facets"
import { metadata } from "@/lib/ipc/metadata"
import { results } from "@/lib/ipc/results"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * The object inspector of the side column: the relation the object view
 * shows. Reads the catalog cache only; the object view loads what is missing.
 */
export function ObjectInspectorPanel({ open }: { open: OpenConnection }) {
  const object = useStore(inspection, (state) =>
    state.object?.connection === open.connection ? state.object : null
  )
  const facets = useQuery({
    queryKey: object
      ? facetsKey(object.connection, object.address)
      : ["relation-facets", "none"],
    enabled: object !== null,
    queryFn: () =>
      object
        ? metadata.relationFacets(object.connection, object.address)
        : Promise.reject(new Error("No object is selected")),
  })
  return (
    <ObjectInspector
      facets={object ? (facets.data ?? null) : null}
      loading={facets.isPending}
      onCopyName={(name) => void copyToClipboard(name, "Qualified name")}
    />
  )
}

/**
 * The record inspector of the side column: the row selected in the grid of
 * this connection — the console's or a preview's. Its cells are one bounded
 * page read from the existing result, never a new query.
 */
export function ValueInspectorPanel({ open }: { open: OpenConnection }) {
  const selected = useStore(inspection, (state) =>
    state.selectedRow?.connection === open.connection ? state.selectedRow : null
  )
  const row = useQuery({
    queryKey: ["inspected-row", selected?.result, selected?.row] as const,
    enabled: selected !== null,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 15_000,
    queryFn: () =>
      selected
        ? results.readResultPage(
            selected.connection,
            selected.result,
            selected.row,
            1
          )
        : Promise.reject(new Error("No row is selected")),
  })
  const cells = row.data?.type === "page" ? (row.data.rows[0] ?? null) : null

  return (
    <ValueInspector
      target={
        selected
          ? { row: selected.row, columns: selected.columns, cells }
          : null
      }
      column={selected?.column ?? null}
      onSelectColumn={selectInspectorColumn}
      onInspect={(column) => {
        if (!selected) return
        inspectValue({
          source: selected.source,
          connection: selected.connection,
          connectionName: selected.connectionName,
          result: selected.result,
          column,
          columnName: selected.columns[column]?.name ?? "",
          row: selected.row,
        })
      }}
    />
  )
}
