import type { OperationKind } from "@/lib/ipc/object-operations"
import type { CatalogNode } from "@/lib/ipc/types"

/**
 * Whether `Drop…`, `Truncate…` and `Rename…` are offered on a catalog node,
 * and why not (docs/adr/0042-revue-sur-place-des-operations-destructrices.md).
 *
 * By the capabilities the catalog's session declares, never by product name
 * (ADR-0003). The backend applies the same rule when it composes, and again
 * on the session it runs on: this copy only decides what the menu greys.
 */
export type OperationOffer =
  /** Not an operation this kind of object has: the entry is absent. */
  | { state: "absent" }
  | { state: "offered" }
  | { state: "greyed"; reason: string }

export const HOSTILE_NAME =
  "This name holds control characters: write the statement in a console."
const NO_DDL = "This connection does not accept schema changes."
const NO_TRUNCATE = "This database has no TRUNCATE statement."

// Control characters (Unicode Cc), line and paragraph separators, and the
// direction controls: shown or escaped, the name would not be the one run.
const HIDDEN = /[\p{Cc}\u2028\u2029\u202A-\u202E\u2066-\u2069]/u

export function hidesText(name: string) {
  return HIDDEN.test(name)
}

function applies(operation: OperationKind, kind: string) {
  if (operation === "drop")
    return kind === "table" || kind === "view" || kind === "materialized_view"
  return kind === "table"
}

export function operationOffer(
  operation: OperationKind,
  node: CatalogNode,
  capabilities: ReadonlyArray<string>
): OperationOffer {
  if (node.address.relation === null || !applies(operation, node.kind))
    return { state: "absent" }
  if (!capabilities.includes("DDL")) return { state: "greyed", reason: NO_DDL }
  if (operation === "truncate" && !capabilities.includes("TRUNCATE"))
    return { state: "greyed", reason: NO_TRUNCATE }
  const segments = [
    node.address.catalog,
    node.address.namespace,
    node.address.relation,
  ]
  if (segments.some((segment) => segment !== null && hidesText(segment)))
    return { state: "greyed", reason: HOSTILE_NAME }
  return { state: "offered" }
}
