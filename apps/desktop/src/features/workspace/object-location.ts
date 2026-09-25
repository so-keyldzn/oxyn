import type { ObjectTab } from "@/components/oxyn/object-view-frame"
import { toast } from "@/components/ui/toast"
import { BackendError } from "@/lib/ipc/client"
import { location } from "@/lib/ipc/location"
import type {
  ObjectPlace,
  SavedObjectPlace,
  SectionChoice,
} from "@/lib/ipc/location"
import type { CatalogAddress } from "@/lib/ipc/types"

export type RelationDirection = "incoming" | "outgoing"

/** The sub-view saved for a tab: Relations says which way its keys point. */
export function sectionOf(
  tab: ObjectTab,
  direction: RelationDirection
): SectionChoice {
  if (tab === "relations")
    return direction === "incoming" ? "incomingRelations" : "relations"
  return tab
}

/** The tab, and the direction when it is Relations, a saved section opens. */
export function tabOf(section: SectionChoice): {
  tab: ObjectTab
  direction?: RelationDirection
} {
  switch (section) {
    case "relations":
      return { tab: "relations", direction: "outgoing" }
    case "incomingRelations":
      return { tab: "relations", direction: "incoming" }
    default:
      return { tab: section }
  }
}

/** How the recovery screen names a saved sub-view: as its tab reads. */
export function sectionLabel(section: SectionChoice) {
  switch (section) {
    case "data":
      return "Data"
    case "structure":
      return "Structure"
    case "indexes":
      return "Indexes"
    case "constraints":
      return "Constraints"
    case "relations":
      return "Relations, outgoing"
    case "incomingRelations":
      return "Relations, incoming"
    case "definition":
      return "DDL"
  }
}

/**
 * The object tab saved last, or `null`. Reads the workspace only, never a
 * server. A failed read costs the restored tab, and is said.
 */
export async function savedObjectPlace(): Promise<SavedObjectPlace | null> {
  try {
    return await location.readObjectLocation()
  } catch (error) {
    toast.add({
      title: "The object open last could not be restored",
      description:
        error instanceof BackendError ? error.message : String(error),
      type: "warning",
    })
    return null
  }
}

/**
 * Saves where browsing stopped on a connection, or forgets it (`null`), for
 * the next launch (ADR-0013, `object_location`).
 *
 * A failed save costs the place at the next launch, never the tab on screen:
 * it is said, and nothing else changes.
 */
export async function saveObjectPlace(
  connection: string,
  place: { address: CatalogAddress; section: SectionChoice } | null
) {
  const saved: ObjectPlace | null = place
  try {
    await location.writeObjectLocation(connection, saved)
  } catch (error) {
    toast.add({
      title: "The open object could not be kept for the next launch",
      description:
        error instanceof BackendError ? error.message : String(error),
      type: "warning",
    })
  }
}
