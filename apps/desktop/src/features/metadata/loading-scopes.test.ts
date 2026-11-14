import { describe, expect, it } from "vitest"
import { loadingScopes } from "./loading-scopes"
import type { CatalogNode } from "@/lib/ipc/types"

const node = (
  catalog: string,
  namespace: string | null,
  children: Array<CatalogNode> = []
): CatalogNode => ({
  address: { catalog, namespace, relation: null },
  name: namespace ?? catalog,
  kind: namespace === null ? "catalog" : "namespace",
  holdsRecords: false,
  system: false,
  comment: null,
  loaded: false,
  stale: false,
  children,
  virtualTable: null,
  shadowOf: null,
})
describe("native preload scopes", () => {
  it("keeps homonymous namespaces and attached SQLite files distinct without inventing unread children", () => {
    const nodes = [
      node("main", null, [node("main", "public")]),
      node("other", null, [node("other", "public")]),
      node("unexpanded", null),
    ]
    const schemas = loadingScopes(nodes, true)
    expect(schemas.map((scope) => scope.scope)).toEqual([
      { catalog: "main", namespace: "public" },
      { catalog: "other", namespace: "public" },
    ])
    expect(new Set(schemas.map((scope) => scope.key)).size).toBe(2)
    expect(
      loadingScopes([node("main", null), node("attached", null)], false).map(
        (scope) => scope.scope
      )
    ).toEqual([
      { catalog: "main", namespace: null },
      { catalog: "attached", namespace: null },
    ])
    expect(loadingScopes([], true)).toEqual([])
    expect(loadingScopes([], false, true).map((scope) => scope.scope)).toEqual([
      { catalog: null, namespace: null },
    ])
    expect(loadingScopes([], false, false)).toEqual([])
  })
})
