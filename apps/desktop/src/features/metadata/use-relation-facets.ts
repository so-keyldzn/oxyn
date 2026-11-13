import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"

import { addressKey } from "@/components/oxyn/catalog-tree"
import { IDLE } from "@/components/oxyn/facet-frame"
import type { FacetLoad } from "@/components/oxyn/facet-frame"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { metadataReads } from "@/features/metadata/read-requests"
import { BackendError, backend } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import type { RelationFacet, RelationFacets } from "@/lib/ipc/metadata"
import type { CatalogAddress, OpenConnection } from "@/lib/ipc/types"

export function facetsKey(
  connection: string,
  address: CatalogAddress,
  session = ""
) {
  return ["relation-facets", connection, session, addressKey(address)] as const
}

const FACETS: Array<RelationFacet> = [
  "detail",
  "constraints",
  "incomingKeys",
  "definition",
]

const idleLoads = (): Record<RelationFacet, FacetLoad> => ({
  detail: IDLE,
  constraints: IDLE,
  incomingKeys: IDLE,
  definition: IDLE,
})

function freshnessOf(facets: RelationFacets, facet: RelationFacet) {
  switch (facet) {
    case "detail":
      return facets.detail.freshness
    case "constraints":
      return facets.constraints.freshness
    case "incomingKeys":
      return facets.incomingKeys.freshness
    case "definition":
      return facets.definition.freshness
  }
}

/**
 * A relation's facets, each loaded when its tab asks and never before.
 *
 * Each facet has its own identity and cancellation: leaving the DDL tab does
 * not cancel the constraints read. A read that fails is shown and not tried
 * again until the user asks, or until a DDL invalidates the catalog — the one
 * case where the tab on screen reads again by itself (ADR-0022).
 */
export function useRelationFacets(
  open: OpenConnection,
  address: CatalogAddress
) {
  const queryClient = useQueryClient()
  const key = facetsKey(open.connection, address, open.session)
  const facets = useQuery({
    queryKey: key,
    queryFn: () =>
      metadata.relationFacets(open.connection, address, open.session),
  })
  const [loads, setLoads] = React.useState(idleLoads)
  const running = React.useRef<Partial<Record<RelationFacet, string>>>({})
  // Facets already asked for since the last invalidation: `ensure` asks once,
  // so a failed read is not repeated in a loop.
  const attempted = React.useRef(new Set<RelationFacet>())
  const invalidated = React.useRef(new Set<RelationFacet>())

  const setLoad = (facet: RelationFacet, load: FacetLoad) =>
    setLoads((current) => ({ ...current, [facet]: load }))

  const refresh = React.useCallback(
    async (facet: RelationFacet, fresh = true) => {
      const { id, outcome: pending } = metadataReads.facet(
        open.connection,
        open.session,
        address,
        facet,
        fresh
      )
      running.current[facet] = id
      attempted.current.add(facet)
      invalidated.current.delete(facet)
      setLoad(facet, { status: "loading" })
      let next: FacetLoad = IDLE
      try {
        const outcome = await pending
        switch (outcome.type) {
          case "cancelled":
            next = { status: "cancelled" }
            break
          case "denied":
            next = { status: "error", message: outcome.reason }
            break
          case "needsApproval":
            // A metadata read waits for nobody: refused, and said.
            void backend.decide(outcome.command, false)
            next = {
              status: "error",
              message:
                "The connection policy requires an approval for this metadata read.",
            }
            break
          default:
            next = IDLE
        }
      } catch (error) {
        next = {
          status: "error",
          message:
            error instanceof BackendError ? error.message : String(error),
          missingModule:
            error instanceof BackendError ? error.missingModule : null,
        }
      }
      if (running.current[facet] !== id) return
      delete running.current[facet]
      setLoad(facet, next)
      await queryClient.invalidateQueries({ queryKey: key })
    },
    // `key` is derived from these two; `address` is compared by its key.
    [open.connection, open.session, key[3], queryClient]
  )

  const cancel = React.useCallback(
    (facet: RelationFacet) => {
      void metadataReads.cancelFacet(
        open.connection,
        open.session,
        address,
        facet
      )
    },
    [open.connection, open.session, key[3]]
  )

  /** Loads a facet the first time a tab shows it, or after an invalidation. */
  const ensure = React.useCallback(
    (facet: RelationFacet) => {
      const data = facets.data
      if (!data || attempted.current.has(facet)) return
      if (
        !invalidated.current.has(facet) &&
        freshnessOf(data, facet).state === "fetched" &&
        !metadataReads.pendingFacet(
          open.connection,
          open.session,
          address,
          facet
        )
      )
        return
      void refresh(facet, invalidated.current.has(facet))
    },
    [facets.data, refresh]
  )

  useRefreshSignal(open.connection, (signal) => {
    if (signal.type === "catalogChanged") {
      void queryClient.invalidateQueries({ queryKey: key })
      return
    }
    if (signal.type !== "catalogInvalidated" && signal.type !== "lagged") return
    // The catalog was invalidated: every facet may be asked for again, and
    // the cache read tells which ones are now stale.
    attempted.current.clear()
    invalidated.current = new Set(FACETS)
    void queryClient.invalidateQueries({ queryKey: key })
  })

  return {
    facets: facets.data ?? null,
    error: facets.error?.message ?? null,
    loads,
    refresh,
    cancel,
    ensure,
  }
}
