// What an `erd` block draws, assembled from the catalog and nothing else.
//
// The block names tables; the model does not describe them. Each name is
// looked up among the objects the backend has read, each table found is read
// for available fields and keys by the shared bounded host map loader. No model is asked anything here, and the same catalog
// draws the same diagram.

import { addressKey } from "@/components/oxyn/catalog-tree"
import type { ErdAmbiguity, ErdState } from "@/components/oxyn/assistant-erd"
import type { ErdName } from "@/components/oxyn/assistant-markdown-model"
import { ERD_MAX_TABLES } from "@/components/oxyn/erd-model"
import { erdProjection } from "@/components/oxyn/schema-map-model"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"
import type { SchemaMapLoaded } from "@/lib/ipc/schema-map"
import type { CatalogAddress } from "@/lib/ipc/types"

/** Reads run at once: a diagram of 40 tables is not 80 commands in flight. */
const CONCURRENCY = 4

export interface ErdSources {
  /** The catalog search: only what the backend has already read. */
  search: (relation: string) => Promise<Array<CatalogSearchHit>>
  /** Same host projection as the standalone map, without implicit neighbour expansion. */
  project: (addresses: Array<CatalogAddress>) => Promise<SchemaMapLoaded>
}

async function inPool<T, TResult>(
  items: Array<T>,
  work: (item: T) => Promise<TResult>
): Promise<Array<TResult>> {
  const out = new Array<TResult>(items.length)
  let next = 0
  const worker = async () => {
    while (next < items.length) {
      const index = next
      next += 1
      out[index] = await work(items[index] as T)
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(CONCURRENCY, items.length) }, worker)
  )
  return out
}

function readable(address: CatalogAddress) {
  return [address.catalog, address.namespace, address.relation]
    .filter((segment) => segment !== null)
    .join(".")
}

type Resolution =
  | { kind: "found"; address: CatalogAddress }
  | { kind: "notFound" }
  | { kind: "ambiguous"; candidates: Array<CatalogAddress> }

/**
 * The table a name designates, if the catalog says so. Exact spelling first;
 * then, since SQL folds unquoted names, a match ignoring case — only if it is
 * the only one. Two candidates are never settled by a guess.
 */
export function resolveName(
  name: ErdName,
  hits: Array<CatalogSearchHit>
): Resolution {
  const relations = hits.filter((hit) => hit.address.relation !== null)
  const pick = (same: (a: string, b: string) => boolean) =>
    relations.filter(
      (hit) =>
        same(hit.address.relation ?? "", name.relation) &&
        (name.namespace === null ||
          same(hit.address.namespace ?? "", name.namespace))
    )
  let matches = pick((a, b) => a === b)
  if (matches.length === 0)
    matches = pick((a, b) => a.toLowerCase() === b.toLowerCase())
  const unique = new Map(matches.map((hit) => [addressKey(hit.address), hit]))
  const found = [...unique.values()]
  if (found.length === 0) return { kind: "notFound" }
  if (found.length > 1)
    return { kind: "ambiguous", candidates: found.map((hit) => hit.address) }
  const [only] = found
  return only ? { kind: "found", address: only.address } : { kind: "notFound" }
}

/** The diagram for these names. Throws what a read threw, unparaphrased. */
export async function assembleErd(
  names: Array<ErdName>,
  ignoredNames: number,
  sources: ErdSources
): Promise<Extract<ErdState, { status: "ready" }>> {
  // One search per distinct spelling: `a.orders` and `b.orders` share it.
  const spellings = [...new Set(names.map((name) => name.relation))]
  const searched = new Map(
    (
      await inPool(spellings, async (relation) => ({
        relation,
        hits: await sources.search(relation),
      }))
    ).map(({ relation, hits }) => [relation, hits])
  )

  const notFound: Array<string> = []
  const ambiguous: Array<ErdAmbiguity> = []
  const starts = new Map<string, CatalogAddress>()
  for (const name of names) {
    const resolution = resolveName(name, searched.get(name.relation) ?? [])
    switch (resolution.kind) {
      case "notFound":
        notFound.push(name.written)
        break
      case "ambiguous":
        ambiguous.push({
          written: name.written,
          candidates: resolution.candidates.map(readable),
        })
        break
      case "found":
        starts.set(addressKey(resolution.address), resolution.address)
    }
  }

  const startList = [...starts.values()].slice(0, ERD_MAX_TABLES)
  const map = startList.length > 0 ? await sources.project(startList) : null
  const graph = map ? erdProjection(map.projection) : { tables: [], links: [] }
  return {
    status: "ready",
    ...graph,
    omitted:
      starts.size -
      startList.length +
      (map?.projection.coverage.omittedObjects ?? 0),
    coverageNotice: map
      ? `${map.projection.coverage.unreadObjects} unread descriptions; ${map.projection.coverage.staleObjects} stale objects; ${map.projection.coverage.omittedFields} omitted fields; ${map.projection.coverage.omittedLinks} omitted links; ${map.projection.coverage.unresolvedLinks} unloaded endpoints; ${map.issues.length} unavailable/failed parts${map.stopped ? "; read stopped" : ""}. Incoming links cover loaded objects only. Expand neighbours in the standalone map.`
      : undefined,
    notFound,
    ambiguous,
    ignoredNames,
  }
}
