import type { RelationFacet, RelationFacets } from "@/lib/ipc/metadata"

/** Availability is separate; these timestamps describe only the named read. */
export function facetFreshness(facets: RelationFacets, facet: RelationFacet) {
  switch (facet) {
    case "detail":
      return facets.aggregateFreshness
    case "description":
    case "sampledDescription":
      return facets.detail.freshness
    case "indexes":
      return facets.indexesFreshness
    case "outgoingKeys":
      return facets.foreignKeysFreshness
    case "constraints":
      return facets.constraints.freshness
    case "incomingKeys":
      return facets.incomingKeys.freshness
    case "definition":
      return facets.definition.freshness
  }
}
