import { useQueryClient } from "@tanstack/react-query"

import { copyToClipboard } from "@/features/metadata/clipboard"
import { facetsKey } from "@/features/metadata/use-relation-facets"
import type { CopyAsForm } from "@/lib/actions/targets"
import { backend, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import type { RelationFacet } from "@/lib/ipc/metadata"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"

/** What the clipboard's toast calls each form of `Copy as ▸`. */
const COPIED_AS: Record<CopyAsForm, string> = {
  quotedName: "Quoted name",
  selectAll: "SELECT *",
  insertTemplate: "INSERT template",
  ddl: "DDL",
}

/** What a cancelled read of each facet is called. */
const FACET_WORDS: Record<RelationFacet, string> = {
  detail: "structure",
  constraints: "constraints",
  incomingKeys: "incoming keys",
  definition: "definition",
}

/**
 * The catalog menu's copies: `Copy qualified name` and `Copy as ▸`.
 *
 * Each one starts in the click and reads its text afterwards (clipboard.ts):
 * WebKit refuses a copy that waited for the backend. A copy that fails says
 * so in its own toast — never as a catalog that could not be read.
 */
export function useCatalogCopies(open: OpenConnection) {
  const queryClient = useQueryClient()

  // The facets the object view reads, with `facet` loaded through the bus
  // when it never was (I-01), and written back under that view's key.
  const facetsWith = async (node: CatalogNode, facet: RelationFacet) => {
    const key = facetsKey(open.connection, node.address)
    const cached = await queryClient.fetchQuery({
      queryKey: key,
      queryFn: () => metadata.relationFacets(open.connection, node.address),
    })
    if (cached[facet].freshness.state === "fetched") return cached
    const outcome = await metadata.refreshRelationFacet(
      newCommandId(),
      open.connection,
      open.session,
      node.address,
      facet
    )
    switch (outcome.type) {
      case "cancelled":
        throw new Error(`Reading the ${FACET_WORDS[facet]} was cancelled.`)
      case "denied":
        throw new Error(outcome.reason)
      case "needsApproval":
        // A metadata read waits for nobody: refused, and said.
        void backend.decide(outcome.command, false)
        throw new Error(
          "The connection policy requires an approval for this metadata read."
        )
      default:
    }
    const read = await metadata.relationFacets(open.connection, node.address)
    queryClient.setQueryData(key, read)
    return read
  }

  // Every form but DDL is composed by the backend, identifiers quoted by the
  // driver (I-10); nothing runs, the text goes to the clipboard. An `INSERT`
  // template names the columns: the structure is read first if it never was.
  const sqlOf = async (node: CatalogNode, form: CopyAsForm) => {
    if (form === "ddl") {
      const definition = (await facetsWith(node, "definition")).definition.value
      if (definition === null)
        throw new Error("No definition was reported for this object.")
      return definition.sql
    }
    if (form === "insertTemplate") await facetsWith(node, "detail")
    return metadata.composeObjectSql(open.connection, node.address, form)
  }

  return {
    copyName: (node: CatalogNode) =>
      void copyToClipboard(
        metadata
          .relationFacets(open.connection, node.address)
          .then((facets) => facets.qualifiedName),
        "Qualified name"
      ),
    copyAs: (node: CatalogNode, form: CopyAsForm) =>
      void copyToClipboard(sqlOf(node, form), COPIED_AS[form]),
  }
}
