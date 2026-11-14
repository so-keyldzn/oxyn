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
import { schemaMaps } from "@/lib/ipc/schema-map"
import { mapReference } from "@/features/schema-map/scopes"
import type { MapObjectRef } from "@/lib/ipc/schema-map"
import type { CatalogAddress, OpenConnection } from "@/lib/ipc/types"

function sourcesFor(
  queryClient: QueryClient,
  open: OpenConnection,
  signal: AbortSignal
): ErdSources {
  return {
    // The sidebar's own key: a search it already ran is not run again.
    search: (relation) =>
      queryClient.fetchQuery({
        queryKey: ["catalog-search", open.connection, open.session, relation],
        queryFn: () =>
          metadata.searchCatalog(open.connection, relation, open.session),
      }),
    project: async (addresses) => {
      if (signal.aborted) throw new Error("The diagram read was cancelled.")
      const id = newCommandId()
      const cancel = () => void backend.cancel(id).catch(() => undefined)
      signal.addEventListener("abort", cancel)
      try {
        return await schemaMaps.load(id, {
          connection: open.connection,
          session: open.session,
          scope: { catalog: null, namespace: null },
          objects: addresses
            .map(mapReference)
            .filter((object): object is MapObjectRef => object !== null),
          expand: null,
          refresh: false,
        })
      } finally {
        signal.removeEventListener("abort", cancel)
      }
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
    open.session,
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
  // Started in the click, the name read afterwards (clipboard.ts).
  const copyName = (table: ErdTable) =>
    void copyToClipboard(
      queryClient
        .fetchQuery({
          queryKey: facetsKey(open.connection, table.address, open.session),
          queryFn: () =>
            metadata.relationFacets(
              open.connection,
              table.address,
              open.session
            ),
        })
        .then(
          (facets) => facets.qualifiedName,
          // The drawn name stays: the copy says what the diagram shows.
          () =>
            table.namespace === null
              ? table.name
              : `${table.namespace}.${table.name}`
        ),
      "Name"
    )

  return (
    <AssistantErd
      source={request.source}
      state={state}
      onOpenObject={onOpenObject}
      onCopyName={copyName}
      // A read, asked again by the user: nothing is written, nothing replays.
      onRetry={() => void erd.refetch()}
    />
  )
}
