import type { ObjectTab } from "@/components/oxyn/object-view-frame"
import type { RelationFacet } from "@/lib/ipc/metadata"
import type { OpenConnection } from "@/lib/ipc/types"
import { queryLanguageOf } from "@/lib/query-language"

/**
 * Why a tab cannot load for this session, or `null` when it can.
 *
 * The same words the backend refuses with (`backend/metadata.rs`), and the
 * same capabilities: the interface is conditional on what the session declares
 * (ADR-0003), and a tab never pretends a source reported « none ».
 */
export function unsupportedReason(
  open: OpenConnection,
  tab: Exclude<ObjectTab, "data" | "structure"> | "incoming"
): string | null {
  const has = (name: string) => open.capabilities.includes(name)
  switch (tab) {
    case "indexes":
      return has("INDEXES")
        ? null
        : "This session does not support index introspection."
    case "constraints":
      return has("CONSTRAINTS")
        ? null
        : "This session does not support constraint introspection."
    case "relations":
      return has("FOREIGN_KEYS")
        ? null
        : "This session does not support foreign key introspection."
    case "incoming":
      return has("INCOMING_FOREIGN_KEYS")
        ? null
        : "This session does not support incoming foreign key discovery."
    case "definition":
      return has("OBJECT_DEFINITION")
        ? null
        : "This session does not provide object definitions."
  }
}

/**
 * The object tabs a session cannot fill, which are not drawn (UX-SPEC
 * § Common workspace structure): a key-value session would otherwise show
 * three tabs that each say "not supported". DDL stays, its own panel saying
 * why it has no definition.
 */
export function absentObjectTabs(open: OpenConnection): ReadonlySet<ObjectTab> {
  const absent = new Set<ObjectTab>(
    (["indexes", "constraints"] as const).filter(
      (tab) => unsupportedReason(open, tab) !== null
    )
  )
  // Relations has two directions: either one is enough to draw it.
  if (
    unsupportedReason(open, "relations") &&
    unsupportedReason(open, "incoming")
  )
    absent.add("relations")
  return absent
}

export function canPreview(open: OpenConnection, holdsRecords: boolean) {
  return previewUnavailable(open, holdsRecords) === null
}

/** Why the Data tab is disabled, in the words the tab's tooltip says. */
export function previewUnavailable(
  open: OpenConnection,
  holdsRecords: boolean
): string | null {
  if (!holdsRecords) return "This object holds no rows to preview."
  // Any language the session speaks: the driver composes the preview in it.
  if (queryLanguageOf(open.capabilities) === null)
    return "This session cannot read rows with a query."
  return null
}

/** The interface's admission hint; the command bus revalidates every read. */
export function facetUnavailable(
  open: OpenConnection,
  facet: RelationFacet
): string | null {
  switch (facet) {
    case "detail":
    case "description":
      return open.capabilities.includes("INFERRED_SCHEMA")
        ? "Schema fields require an explicit bounded sampling request."
        : null
    case "sampledDescription":
      return open.capabilities.includes("INFERRED_SCHEMA")
        ? null
        : "This session does not provide schema inference."
    case "indexes":
      return unsupportedReason(open, "indexes")
    case "outgoingKeys":
      return unsupportedReason(open, "relations")
    case "constraints":
      return unsupportedReason(open, "constraints")
    case "incomingKeys":
      return unsupportedReason(open, "incoming")
    case "definition":
      return unsupportedReason(open, "definition")
  }
}
