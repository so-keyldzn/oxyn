import type { LoadingScope } from "@/components/oxyn/metadata-loading-dialog"
import type { CatalogNode } from "@/lib/ipc/types"

/** Only observed native levels are offered; choosing one never discovers siblings. */
export function loadingScopes(
  nodes: Array<CatalogNode>,
  schemas: boolean,
  rootObserved = false
): Array<LoadingScope> {
  const found = new Map<string, LoadingScope>()
  const visit = (items: Array<CatalogNode>) => {
    for (const node of items) {
      if (
        (schemas && node.kind === "namespace") ||
        (!schemas && node.kind === "catalog")
      ) {
        const scope = {
          catalog: node.address.catalog,
          namespace: node.address.namespace,
        }
        const key = JSON.stringify(scope)
        found.set(key, {
          key,
          scope,
          label:
            [scope.catalog, scope.namespace]
              .filter((name) => name !== null)
              .join(" / ") || "Source",
        })
      }
      visit(node.children)
    }
  }
  visit(nodes)
  if (!schemas && rootObserved && found.size === 0) {
    const scope = { catalog: null, namespace: null }
    const key = JSON.stringify(scope)
    found.set(key, { key, scope, label: "Source" })
  }
  return [...found.values()]
}
