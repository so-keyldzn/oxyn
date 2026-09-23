// What an `erd` block draws, assembled from the catalog and nothing else.
//
// The block names tables; the model does not describe them. Each name is
// looked up among the objects the backend has read, each table found is read
// for its columns and keys, and its direct neighbours by a foreign key are
// added under a bound. No model is asked anything here, and the same catalog
// draws the same diagram.

import { addressKey } from "@/components/oxyn/catalog-tree"
import type { ErdAmbiguity, ErdState } from "@/components/oxyn/assistant-erd"
import type { ErdName } from "@/components/oxyn/assistant-markdown-model"
import { ERD_MAX_TABLES } from "@/components/oxyn/erd-diagram"
import type { ErdLink, ErdTable } from "@/components/oxyn/erd-diagram"
import type {
  CatalogSearchHit,
  RelationFacet,
  RelationFacets,
} from "@/lib/ipc/metadata"
import type { CatalogAddress } from "@/lib/ipc/types"

/** Reads run at once: a diagram of 40 tables is not 80 commands in flight. */
const CONCURRENCY = 4

export interface ErdSources {
  /** The catalog search: only what the backend has already read. */
  search: (relation: string) => Promise<Array<CatalogSearchHit>>
  /** A relation's facets, with those named read first if never read. */
  facets: (
    address: CatalogAddress,
    wanted: Array<RelationFacet>
  ) => Promise<RelationFacets>
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
  return [address.namespace, address.relation]
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

function tableOf(facets: RelationFacets, requested: boolean): ErdTable {
  const keyed = new Set((facets.foreignKeys ?? []).flatMap((key) => key.fields))
  const fields = [...(facets.detail.value?.fields ?? [])].sort(
    (a, b) => a.position - b.position
  )
  return {
    key: addressKey(facets.address),
    address: facets.address,
    name: facets.detail.value?.name ?? facets.address.relation ?? "",
    namespace: facets.address.namespace,
    columns: fields.map((field) => ({
      name: field.name,
      type: field.rawType,
      primaryKey: field.primaryKey,
      foreignKey: keyed.has(field.name),
    })),
    requested,
  }
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
  const startFacets = await inPool(startList, (address) =>
    sources.facets(address, ["detail", "incomingKeys"])
  )

  const neighbours = new Map<string, CatalogAddress>()
  for (const facets of startFacets) {
    const related = [
      ...(facets.foreignKeys ?? []).map((key) => key.references),
      ...(facets.incomingKeys.value ?? []).map((incoming) => incoming.source),
    ]
    for (const address of related) {
      const key = addressKey(address)
      if (!starts.has(key) && !neighbours.has(key)) neighbours.set(key, address)
    }
  }
  const room = Math.max(ERD_MAX_TABLES - startList.length, 0)
  const neighbourList = [...neighbours.values()].slice(0, room)
  const omitted =
    neighbours.size - neighbourList.length + (starts.size - startList.length)
  const neighbourFacets = await inPool(neighbourList, (address) =>
    sources.facets(address, ["detail"])
  )

  const tables = [
    ...startFacets.map((facets) => tableOf(facets, true)),
    ...neighbourFacets.map((facets) => tableOf(facets, false)),
  ]
  const drawn = new Set(tables.map((table) => table.key))
  const links: Array<ErdLink> = []
  const seen = new Set<string>()
  // Every drawn table's own outgoing keys: those between two neighbours are
  // real keys too, and leaving them out would draw a false absence.
  for (const facets of [...startFacets, ...neighbourFacets]) {
    const from = addressKey(facets.address)
    for (const key of facets.foreignKeys ?? []) {
      const to = addressKey(key.references)
      const id = JSON.stringify([from, key.name])
      if (!drawn.has(to) || seen.has(id)) continue
      seen.add(id)
      links.push({
        key: id,
        from,
        to,
        name: key.name,
        fields: key.fields,
        referencedFields: key.referencedFields,
      })
    }
  }

  return {
    status: "ready",
    tables,
    links,
    omitted,
    notFound,
    ambiguous,
    ignoredNames,
  }
}
