import type { MapScopeChoice } from "@/components/oxyn/schema-map-view"
import type { CatalogAddress, CatalogNode } from "@/lib/ipc/types"
import type { MapObjectRef } from "@/lib/ipc/schema-map"
import type { MetadataScope } from "@/lib/ipc/metadata-loading"

export const scopeKey = (scope: MetadataScope) => JSON.stringify(scope)
/** Includes native containers so DuckDB catalogs can be read one level at a time. */
export function mapScopes(
  nodes: Array<CatalogNode>,
  schemas: boolean
): Array<MapScopeChoice> {
  const choices = new Map<string, MapScopeChoice>()
  const visit = (items: Array<CatalogNode>) => {
    for (const node of items) {
      if (node.kind === "catalog" || node.kind === "namespace") {
        const scope = {
          catalog: node.address.catalog,
          namespace: node.address.namespace,
        }
        const key = scopeKey(scope)
        choices.set(key, {
          key,
          scope,
          label:
            [scope.catalog, scope.namespace]
              .filter((part) => part !== null)
              .join(" / ") || "Source root",
        })
      }
      visit(node.children)
    }
  }
  visit(nodes)
  if (!schemas && choices.size === 0) {
    const scope = { catalog: null, namespace: null }
    const key = scopeKey(scope)
    choices.set(key, { key, scope, label: "Source root" })
  }
  return [...choices.values()]
}
export function mapReference(address: CatalogAddress): MapObjectRef | null {
  return address.relation === null
    ? null
    : {
        scope: { catalog: address.catalog, namespace: address.namespace },
        relation: address.relation,
      }
}
