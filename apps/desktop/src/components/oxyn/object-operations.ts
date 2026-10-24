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
const NOT_KNOWN =
  "No session here to run it: do it from the object in the workspace's catalog."

// Control and format characters (Unicode Cc and Cf: direction controls,
// zero-width characters), line and paragraph separators, and the four fillers
// that render blank: shown or escaped, the name would not be the one run. The
// same set as `oxyn_catalog::hides_text`, which the backend applies.
const HIDDEN = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\u115F\u1160\u3164\uFFA0]/u

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
  capabilities: ReadonlyArray<string> | null
): OperationOffer {
  if (node.address.relation === null || !applies(operation, node.kind))
    return { state: "absent" }
  // No session to read them from: the entry stays, greyed, as on a sibling
  // whose session is known.
  if (capabilities === null) return { state: "greyed", reason: NOT_KNOWN }
  // The statement is SQL: an Elasticsearch alias is a view of the catalog,
  // and `DROP VIEW` means nothing to its cluster.
  if (!capabilities.includes("SQL")) return { state: "absent" }
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
