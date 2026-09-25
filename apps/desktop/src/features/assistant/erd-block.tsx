import { useQuery, useQueryClient } from "@tanstack/react-query"
import type { QueryClient } from "@tanstack/react-query"

import { AssistantErd } from "@/components/oxyn/assistant-erd"
import type { ErdState } from "@/components/oxyn/assistant-erd"
import type { ErdRequest } from "@/components/oxyn/assistant-markdown-model"
import type { ErdTable } from "@/components/oxyn/erd-model"
import { assembleErd } from "@/features/assistant/erd"
import type { ErdSources } from "@/features/assistant/erd"
import { copyToClipboard } from "@/features/metadata/clipboard"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { facetsKey } from "@/features/metadata/use-relation-facets"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import type { RelationFacet, RelationFacets } from "@/lib/ipc/metadata"
import type { CatalogAddress, OpenConnection } from "@/lib/ipc/types"

function freshness(facets: RelationFacets, facet: RelationFacet) {
  switch (facet) {
    case "detail":
      return facets.detail.freshness.state
    case "constraints":
      return facets.constraints.freshness.state
    case "incomingKeys":
      return facets.incomingKeys.freshness.state
    case "definition":
      return facets.definition.freshness.state
  }
}

/**
 * Reads a facet never read, through the same command as the object view.
 * A metadata read waits for nobody: one that needs an approval is refused
 * and said, as `useRelationFacets` does.
 */
async function readFacet(
  open: OpenConnection,
  address: CatalogAddress,
  facet: RelationFacet,
  signal: AbortSignal
) {
  const id = newCommandId()
  const cancel = () => void backend.cancel(id)
  signal.addEventListener("abort", cancel)
  try {
    const outcome = await metadata.refreshRelationFacet(
      id,
      open.connection,
      open.session,
      address,
      facet
    )
    switch (outcome.type) {
      case "cancelled":
        throw new Error("The metadata read was cancelled.")
      case "denied":
        throw new Error(outcome.reason)
      case "needsApproval":
        void backend.decide(outcome.command, false)
        throw new Error(
          "The connection policy requires an approval for this metadata read."
        )
      default:
    }
  } finally {
    signal.removeEventListener("abort", cancel)
  }
}

function sourcesFor(
  queryClient: QueryClient,
  open: OpenConnection,
  signal: AbortSignal
): ErdSources {
  return {
    // The sidebar's own key: a search it already ran is not run again.
    search: (relation) =>
      queryClient.fetchQuery({
        queryKey: ["catalog-search", open.connection, relation],
        queryFn: () => metadata.searchCatalog(open.connection, relation),
      }),
    facets: async (address, wanted) => {
      const key = facetsKey(open.connection, address)
      const cached = await queryClient.fetchQuery({
        queryKey: key,
        queryFn: () => metadata.relationFacets(open.connection, address),
      })
      const missing = wanted.filter(
        (facet) => freshness(cached, facet) !== "fetched"
      )
      if (missing.length === 0) return cached
      for (const facet of missing) await readFacet(open, address, facet, signal)
      // Written back under the object view's key: it now reads fresh too.
      const read = await metadata.relationFacets(open.connection, address)
      queryClient.setQueryData(key, read)
      return read
    },
  }
}

/**
 * An `erd` block, drawn from this connection's catalog.
 *
 * The names are resolved and the tables read by the metadata commands the
 * catalog uses; nothing here is sent to a model, and nothing runs a query.
 */
export function ErdBlock({
  open,
  request,
  onOpenObject,
}: {
  open: OpenConnection
  request: ErdRequest
  onOpenObject?: (address: CatalogAddress) => void
}) {
  const queryClient = useQueryClient()
  const erdKey = [
    "assistant-erd",
    open.connection,
    JSON.stringify(
      request.names.map((name) => [name.namespace, name.relation])
    ),
  ] as const
  const erd = useQuery({
    queryKey: erdKey,
    queryFn: ({ signal }) =>
      assembleErd(
        request.names,
        request.ignoredNames,
        sourcesFor(queryClient, open, signal)
      ),
    // A diagram in a conversation is read, not polled: it is read again when
    // the catalog changes, below, or when the user asks.
    staleTime: Infinity,
  })

  useRefreshSignal(open.connection, (signal) => {
    if (signal.type !== "catalogInvalidated" && signal.type !== "lagged") return
    void queryClient.invalidateQueries({
      queryKey: ["assistant-erd", open.connection],
    })
  })

  const state: ErdState = erd.isPending
    ? { status: "loading" }
    : erd.isError
      ? {
          status: "error",
          message:
            erd.error instanceof BackendError || erd.error instanceof Error
              ? erd.error.message
              : String(erd.error),
        }
      : erd.data

  // The name the backend quoted for the session's dialect, from the facets
  // the diagram was drawn from; the name as drawn when they cannot be read.
  // Never a name composed here (I-10).
  const copyName = async (table: ErdTable) => {
    let name =
      table.namespace === null ? table.name : `${table.namespace}.${table.name}`
    try {
      const facets = await queryClient.fetchQuery({
        queryKey: facetsKey(open.connection, table.address),
        queryFn: () => metadata.relationFacets(open.connection, table.address),
      })
      name = facets.qualifiedName
    } catch {
      // The drawn name stays: the copy says what the diagram shows.
    }
    await copyToClipboard(name, "Name")
  }

  return (
    <AssistantErd
      source={request.source}
      state={state}
      onOpenObject={onOpenObject}
      onCopyName={(table) => void copyName(table)}
      // A read, asked again by the user: nothing is written, nothing replays.
      onRetry={() => void erd.refetch()}
    />
  )
}
