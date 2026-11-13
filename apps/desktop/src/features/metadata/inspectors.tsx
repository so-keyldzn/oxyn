import { useQuery } from "@tanstack/react-query"
import { useSelector } from "@tanstack/react-store"

import { ObjectInspector } from "@/components/oxyn/object-inspector"
import { ValueInspector } from "@/components/oxyn/value-inspector"
import type { InspectedRowState } from "@/components/oxyn/value-inspector"
import { RESULT_PAGE_QUERY } from "@/components/oxyn/result-grid"
import { copyToClipboard } from "@/features/metadata/clipboard"
import {
  inspectValue,
  inspection,
  selectInspectorColumn,
} from "@/features/metadata/inspection"
import { facetsKey } from "@/features/metadata/use-relation-facets"
import { metadata } from "@/lib/ipc/metadata"
import { BackendError } from "@/lib/ipc/client"
import { results } from "@/lib/ipc/results"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * The object inspector of the side column: the relation the object view
 * shows. Reads the catalog cache only; the object view loads what is missing.
 */
export function ObjectInspectorPanel({ open }: { open: OpenConnection }) {
  const object = useSelector(inspection, (state) =>
    state.object?.connection === open.connection ? state.object : null
  )
  const facets = useQuery({
    queryKey: object
      ? facetsKey(
          object.connection,
          object.address,
          object.session ?? open.session
        )
      : ["relation-facets", "none"],
    enabled: object !== null,
    queryFn: () =>
      object
        ? metadata.relationFacets(
            object.connection,
            object.address,
            object.session ?? open.session
          )
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
  const selected = useSelector(inspection, (state) =>
    state.selectedRow?.connection === open.connection ? state.selectedRow : null
  )
  const row = useQuery({
    queryKey: [
      RESULT_PAGE_QUERY,
      "inspector",
      open.connection,
      selected?.result,
      selected?.row,
    ] as const,
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
  const cells = row.data?.type === "page" ? row.data.rows[0] : undefined
  // An error wins over previously cached data: it must not look current.
  const state: InspectedRowState = row.isError
    ? {
        status: "error",
        error: {
          message: row.error.message,
          retryable: row.error instanceof BackendError && row.error.retryable,
        },
      }
    : row.isPending
      ? { status: "loading" }
      : cells
        ? { status: "loaded", cells }
        : { status: "unavailable" }

  return (
    <ValueInspector
      target={
        selected
          ? { row: selected.row, columns: selected.columns, state }
          : null
      }
      column={selected?.column ?? null}
      onSelectColumn={selectInspectorColumn}
      onRetry={() => void row.refetch()}
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
