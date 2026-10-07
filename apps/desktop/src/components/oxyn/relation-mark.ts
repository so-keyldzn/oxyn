import type { CatalogNode } from "@/lib/ipc/types"

/** What the tree shows beside a relation that is not an ordinary table. */
export type RelationMark = {
  /** The short mark on the row. */
  label: string
  /** The sentence the hint and the screen reader add. */
  description: string
  /** The table cannot be read in this connection. */
  warning: boolean
}

/**
 * The mark of a virtual table or of one of its shadow tables, `null`
 * otherwise. Pure, so it is tested without a DOM.
 *
 * The module name comes from the database: it is shown as text, never run.
 */
export function relationMark(
  node: Pick<CatalogNode, "virtualTable" | "shadowOf">
): RelationMark | null {
  const table = node.virtualTable
  if (table) {
    const module = table.module || "unknown module"
    switch (table.available) {
      case false:
        return {
          label: `${module} not loaded`,
          description: `Virtual table of the SQLite module ${module}, which this connection does not load: its rows cannot be read here.`,
          warning: true,
        }
      case true:
        return {
          label: `virtual · ${module}`,
          description: `Virtual table: its rows are computed by the SQLite module ${module}.`,
          warning: false,
        }
      default:
        return {
          label: `virtual · ${module}`,
          description: `Virtual table of the SQLite module ${module}. Whether this connection loads it is unknown.`,
          warning: false,
        }
    }
  }
  if (node.shadowOf !== null) {
    // The owner is in the hint: on the row, it would push the table's own name
    // out of a narrow sidebar.
    return {
      label: "shadow",
      description: `Stores the data of the virtual table ${node.shadowOf}.`,
      warning: false,
    }
  }
  return null
}
