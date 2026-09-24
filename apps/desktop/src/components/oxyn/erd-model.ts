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
  name: string
  /** As the server reports it, already a string: never parsed here. */
  type: string
  primaryKey: boolean
  foreignKey: boolean
}

export interface ErdTable {
  /** Stable identity, `addressKey` of the address. */
  key: string
  address: CatalogAddress
  name: string
  namespace: string | null
  columns: Array<ErdColumn>
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
}
