import * as React from "react"
import {
  keepPreviousData,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"
import { useDebouncedValue } from "@tanstack/react-pacer"

import type {
  MentionChoice,
  MentionKind,
  MentionResults,
  MentionSource,
} from "@/components/oxyn/assistant-composer"
import { library } from "@/lib/ipc/library"
import type { DocumentEntry } from "@/lib/ipc/library"
import { metadata } from "@/lib/ipc/metadata"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"
import type { CatalogNode } from "@/lib/ipc/types"
import { newCommandId } from "@/lib/ipc/client"

/** Enough to choose from; past eight rows the list scrolls. */
const MAX_CHOICES = 12
/** Saved queries asked of the library, after the catalog's objects. */
const MAX_SAVED = 4
/** Columns offered per relation whose fields matched. */
const MAX_COLUMNS_PER_RELATION = 3
/**
 * The library is a store read, slower than the catalog cache: it is asked a
 * little after the keystroke, and never holds the list back. The catalog is
 * asked at once — the list must answer within 100 ms (docs/PERFORMANCE.md).
 */
const SAVED_DEBOUNCE_MS = 60

function kindOf(kind: string): MentionKind {
  switch (kind) {
    case "table":
      return "table"
    case "view":
    case "materialized_view":
      return "view"
    case "collection":
      return "collection"
    default:
      return "other"
  }
}

function keyOf(hit: CatalogSearchHit, field: string | null) {
  // A JSON array: no separator a hostile name could forge.
  return JSON.stringify([
    hit.address.catalog,
    hit.address.namespace,
    hit.address.relation,
    field,
  ])
}

/**
 * What the text after `@` asks: `orders` searches every object; `orders.st`
 * searches the columns starting with `st` of the relations matching `orders`.
 */
export function readQuery(query: string): {
  search: string
  relation: string | null
  column: string | null
} {
  const dot = query.lastIndexOf(".")
  if (dot < 0) return { search: query, relation: null, column: null }
  const relation = query.slice(0, dot).toLowerCase()
  const column = query.slice(dot + 1).toLowerCase()
  // Both words go to the catalog search: the relation's name and the
  // column's both score, and the column comes back in `matchedFields`.
  return {
    search: [relation, column].filter((word) => word !== "").join(" "),
    relation,
    column,
  }
}

/**
 * The `@` list for one search: the catalog's objects in the backend's order,
 * each followed by its matching columns — those already read into the cache —,
 * then the saved queries kept for this connection or for none.
 *
 * Nothing is ranked here — the catalog search ranks (`oxyn_catalog::search`),
 * the library lists newest first; this only shapes and bounds. A query saved
 * for another connection is left out: the backend would ignore it anyway,
 * since its literals are not this connection's to send.
 */
export function mentionChoices(
  connection: string,
  query: string,
  hits: ReadonlyArray<CatalogSearchHit>,
  saved: ReadonlyArray<DocumentEntry>
): Array<MentionChoice> {
  const { relation: wantedRelation, column: wantedColumn } = readQuery(query)
  const choices: Array<MentionChoice> = []
  for (const hit of hits) {
    if (hit.address.relation === null) continue
    const detail = hit.address.namespace ?? hit.address.catalog
    const name = hit.name.toLowerCase()
    if (wantedRelation === null) {
      // A relation that matched by one of its columns only is offered by
      // that column: the table itself is not what was typed.
      if (hit.matched !== "fieldName")
        choices.push({
          key: keyOf(hit, null),
          kind: kindOf(hit.kind),
          label: hit.name,
          detail,
          mention: { kind: "relation", address: hit.address, field: null },
        })
    } else if (!name.includes(wantedRelation)) continue
    const columns = hit.matchedFields.filter(
      (field) =>
        wantedColumn === null || field.toLowerCase().startsWith(wantedColumn)
    )
    for (const field of columns.slice(0, MAX_COLUMNS_PER_RELATION)) {
      choices.push({
        key: keyOf(hit, field),
        kind: "column",
        label: `${hit.name}.${field}`,
        detail,
        mention: { kind: "relation", address: hit.address, field },
      })
    }
  }
  const kept =
    wantedRelation === null
      ? saved.filter(
          (entry) =>
            entry.isSaved &&
            (entry.connection === null || entry.connection === connection)
        )
      : []
  // Room is kept for the saved queries, which would otherwise never show
  // under a long list of columns.
  const objects = choices.slice(
    0,
    MAX_CHOICES - Math.min(MAX_SAVED, kept.length)
  )
  const queries = kept
    .slice(0, MAX_CHOICES - objects.length)
    .map((entry): MentionChoice => ({
      key: `saved:${entry.id}`,
      kind: "savedQuery",
      label: entry.title,
      detail: null,
      mention: { kind: "savedQuery", document: entry.id },
    }))
  return [...objects, ...queries]
}

/**
 * The objects the catalog tree already holds, in the tree's order: what `@`
 * alone shows before anything is typed.
 *
 * Nothing is ranked or searched here — the tree is the backend's, read from
 * the cache the sidebar fills (`catalog_tree`). Only its relations are kept,
 * system ones left out as the backend flags them, and the walk stops once the
 * list is full. A relation's own children are its fields: not descended into.
 */
export function treeHits(
  nodes: ReadonlyArray<CatalogNode>,
  limit = MAX_CHOICES
): Array<CatalogSearchHit> {
  const hits: Array<CatalogSearchHit> = []
  const walk = (level: ReadonlyArray<CatalogNode>) => {
    for (const node of level) {
      if (hits.length >= limit || node.system) continue
      if (node.address.relation !== null) {
        hits.push({
          address: node.address,
          name: node.name,
          kind: node.kind,
          holdsRecords: node.holdsRecords,
          matched: "relationName",
          matchedFields: [],
        })
      } else walk(node.children)
    }
  }
  walk(nodes)
  return hits
}

/** The same key as the sidebar's tree: one cache, filled by whoever asks first. */
const treeKey = (connection: string) => ["catalog", connection] as const
const savedKey = (search: string) =>
  ["assistant", "mentions", "saved", search] as const

function listSaved(search: string) {
  return library.listDocuments(newCommandId(), {
    savedOnly: true,
    openOnly: false,
    search,
    before: null,
    limit: MAX_SAVED,
  })
}

/**
 * The `@` list of the assistant on `connection`: what the user types after
 * `@`, searched in the local catalog and library, never through a model
 * (`.claude/rules/ia.md`).
 *
 * Called when the panel opens, it warms what the first `@` reads: the
 * catalog tree — the sidebar's own cache, usually already there — and the
 * newest saved queries. `@` alone then shows the tree's objects at once; a
 * typed name asks the catalog search at the keystroke, and the library 60 ms
 * later, between two keystrokes only. What was shown stays shown while the
 * next answer is read, marked as completing, so the list never blinks and
 * never says "No matching object" before an answer said so.
 */
export function useMentionSearch(connection: string): MentionSource {
  const queryClient = useQueryClient()
  const [query, setQuery] = React.useState<string | null>(null)
  const [debounced] = useDebouncedValue(query, { wait: SAVED_DEBOUNCE_MS })
  // The first `@` waits for nothing: only a typed name is debounced.
  const savedQuery = query === "" ? "" : debounced
  const search = query === null ? null : readQuery(query).search

  React.useEffect(() => {
    void queryClient.prefetchQuery({
      queryKey: savedKey(""),
      queryFn: () => listSaved(""),
    })
  }, [queryClient])

  // Stale answers are fine here: the sidebar refreshes this tree and
  // invalidates it; the list only reads it, and asks nothing again itself.
  const tree = useQuery({
    queryKey: treeKey(connection),
    queryFn: () => metadata.catalogTree(connection),
    staleTime: Infinity,
  })
  const catalog = useQuery({
    queryKey: ["assistant", "mentions", "catalog", connection, search],
    queryFn: () => metadata.searchCatalog(connection, search ?? ""),
    enabled: search !== null && search !== "",
    placeholderData: keepPreviousData,
  })
  const saved = useQuery({
    queryKey: savedKey(savedQuery ?? ""),
    queryFn: () => listSaved(savedQuery ?? ""),
    enabled: savedQuery !== null && !savedQuery.includes("."),
    placeholderData: keepPreviousData,
  })

  const bare = query === ""
  const hits = bare
    ? tree.data === undefined
      ? undefined
      : treeHits(tree.data)
    : catalog.data
  const entries = saved.data?.entries
  // The tree is a shortcut: when it cannot be read, `@` alone just invites
  // to type, and its failure belongs to the sidebar.
  const failure = bare ? null : (catalog.error ?? saved.error)
  const treeUnavailable = bare && tree.isError
  const reading = !bare && catalog.isPlaceholderData
  // A column search asks no saved query: nothing is left to complete.
  const completing =
    query !== null &&
    (reading ||
      (!query.includes(".") &&
        (saved.fetchStatus !== "idle" || savedQuery !== query)))
  const shown = React.useRef<ReadonlyArray<MentionChoice> | null>(null)
  const results = React.useMemo((): MentionResults => {
    if (query === null || treeUnavailable) return { status: "idle" }
    if (failure) return { status: "failed", message: failure.message }
    if (hits === undefined)
      return shown.current === null
        ? { status: "loading" }
        : { status: "ready", choices: shown.current, completing: true }
    return {
      status: "ready",
      choices: mentionChoices(connection, query, hits, entries ?? []),
      completing,
    }
  }, [connection, query, failure, treeUnavailable, hits, entries, completing])
  if (results.status === "ready") shown.current = results.choices
  if (query === null) shown.current = null

  return { results, onQuery: setQuery }
}
