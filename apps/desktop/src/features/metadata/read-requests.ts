import { addressKey } from "@/components/oxyn/catalog-tree"
import { backend, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import type { RelationFacet } from "@/lib/ipc/metadata"
import type { CatalogAddress, CommandOutcome } from "@/lib/ipc/types"

interface MetadataRead {
  id: string
  outcome: Promise<CommandOutcome>
}

interface Scope {
  kind: "catalog" | "facet"
  connection: string
  session: string
  key: string
}

interface Pending extends MetadataRead {
  scope: Scope
  started: boolean
  cancelled: boolean
  previous: Pending | undefined
}

// Client-side command ownership outlives React viewers. Remounts join a read;
// an invalidation queues a fresh read without killing a private SQL session.
const pending = new Map<string, Pending>()

function begin(
  scope: Scope,
  execute: (id: string) => Promise<CommandOutcome>,
  fresh: boolean
): MetadataRead {
  const previous = pending.get(scope.key)
  if (previous && (!fresh || !previous.started)) return previous
  const id = newCommandId()
  let resolve!: (outcome: CommandOutcome) => void
  let reject!: (error: unknown) => void
  const outcome = new Promise<CommandOutcome>((yes, no) => {
    resolve = yes
    reject = no
  })
  const read: Pending = {
    id,
    outcome,
    scope,
    started: false,
    cancelled: false,
    previous,
  }
  pending.set(scope.key, read)
  const run = async (): Promise<CommandOutcome> => {
    if (previous) await previous.outcome.catch(() => undefined)
    read.previous = undefined
    if (read.cancelled) return { type: "cancelled" }
    read.started = true
    return execute(id)
  }
  void run().then(resolve, reject)
  const finished = () => {
    if (pending.get(scope.key) === read) pending.delete(scope.key)
  }
  void outcome.then(finished, finished)
  return read
}

async function cancel(read: Pending): Promise<void> {
  read.cancelled = true
  await Promise.all([
    read.previous ? cancel(read.previous) : undefined,
    read.started ? backend.cancel(read.id) : undefined,
  ])
}

function facetKey(
  connection: string,
  session: string,
  address: CatalogAddress,
  facet: RelationFacet
) {
  return JSON.stringify([
    "facet",
    connection,
    session,
    addressKey(address),
    facet,
  ])
}

export const metadataReads = {
  catalog(
    connection: string,
    session: string,
    address: CatalogAddress | null,
    fresh = false
  ) {
    return begin(
      {
        kind: "catalog",
        connection,
        session,
        key: JSON.stringify([
          "catalog",
          connection,
          session,
          address ? addressKey(address) : null,
        ]),
      },
      (id) => metadata.refreshCatalog(id, connection, session, address),
      fresh
    )
  },
  facet(
    connection: string,
    session: string,
    address: CatalogAddress,
    facet: RelationFacet,
    fresh = false
  ) {
    return begin(
      {
        kind: "facet",
        connection,
        session,
        key: facetKey(connection, session, address, facet),
      },
      (id) =>
        metadata.refreshRelationFacet(id, connection, session, address, facet),
      fresh
    )
  },
  pendingFacet(
    connection: string,
    session: string,
    address: CatalogAddress,
    facet: RelationFacet
  ): MetadataRead | undefined {
    return pending.get(facetKey(connection, session, address, facet))
  },
  async cancelCatalog(connection: string, session: string): Promise<void> {
    await Promise.all(
      [...pending.values()]
        .filter(
          ({ scope }) =>
            scope.kind === "catalog" &&
            scope.connection === connection &&
            scope.session === session
        )
        .map(cancel)
    )
  },
  async cancelFacet(
    connection: string,
    session: string,
    address: CatalogAddress,
    facet: RelationFacet
  ): Promise<void> {
    const read = pending.get(facetKey(connection, session, address, facet))
    if (read) await cancel(read)
  },
}
