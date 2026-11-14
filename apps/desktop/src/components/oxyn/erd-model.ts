import type { CatalogAddress } from "@/lib/ipc/types"

// Apart from `erd-diagram.tsx` so that naming a diagram's shape does not pull
// xyflow and dagre into the main bundle: the drawing is loaded on demand.

/**
 * Tables one diagram draws. Past this a diagram is a texture, not a map, and
 * every table costs a metadata read: the rest is counted and said.
 */
export const ERD_MAX_TABLES = 40

/** Columns a table shows; keys first, the rest counted. */
export const ERD_MAX_COLUMNS = 12

export interface ErdColumn {
  key?: string
  name: string
  /** As the server reports it, already a string: never parsed here. */
  type: string
  primaryKey: boolean
  foreignKey: boolean
  origin?: "reported" | "sampled" | "previewLayout"
  depth?: number
}

export interface ErdTable {
  /** Stable identity, `addressKey` of the address. */
  key: string
  address: CatalogAddress
  name: string
  namespace: string | null
  catalog?: string | null
  kind?: string
  fieldsState?: "fresh" | "stale" | "unread"
  columns: Array<ErdColumn>
  /** False distinguishes unread/sample-required structure from known empty. */
  columnsKnown?: boolean
  /** Named by the answer, as opposed to drawn as a neighbour of one. */
  requested: boolean
}

/** A foreign key, drawn from the table that holds it to the one it references. */
export interface ErdLink {
  key: string
  from: string
  to: string
  name: string
  fields: Array<string>
  referencedFields: Array<string>
  stale?: boolean
}

/** The columns drawn: keys first, in position order, bounded. Pure, so tested. */
export function visibleColumns(columns: Array<ErdColumn>) {
  const keys = columns.filter(
    (column) => column.primaryKey || column.foreignKey
  )
  const rest = columns.filter(
    (column) => !column.primaryKey && !column.foreignKey
  )
  const shown = [...keys, ...rest].slice(0, ERD_MAX_COLUMNS)
  return { shown, hidden: columns.length - shown.length }
}
