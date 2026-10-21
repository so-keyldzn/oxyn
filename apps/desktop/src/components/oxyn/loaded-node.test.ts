import { describe, expect, it } from "vitest"

import {
  findLoadedNode,
  tabsWithLoadedFacts,
  withLoadedFacts,
} from "./loaded-node"
import { missingModuleTable, virtualCatalog } from "./virtual-table-fixtures"
import type { CatalogNode } from "@/lib/ipc/types"

/** What an opening by address alone knows: the address, nothing else. */
const byAddress = (relation: string): CatalogNode => ({
  address: { catalog: null, namespace: "main", relation },
  name: relation,
  kind: "table",
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: false,
  stale: false,
  children: [],
  virtualTable: null,
  shadowOf: null,
})

describe("withLoadedFacts", () => {
  it("finds a relation under its namespace", () => {
    expect(
      findLoadedNode(virtualCatalog, missingModuleTable.address)?.virtualTable
    ).toEqual(missingModuleTable.virtualTable)
    expect(
      findLoadedNode(virtualCatalog, {
        catalog: null,
        namespace: "main",
        relation: "absent",
      })
    ).toBeNull()
  })

  it("gives an address-only opening the module of its virtual table", () => {
    const hydrated = withLoadedFacts(byAddress("chunks_vec"), virtualCatalog)
    expect(hydrated.virtualTable).toEqual({
      module: "vec0",
      available: false,
      shadows: expect.arrayContaining(["chunks_vec_info"]),
    })
    expect(hydrated.loaded).toBe(true)
    expect(
      withLoadedFacts(byAddress("chunks_vec_info"), virtualCatalog).shadowOf
    ).toBe("chunks_vec")
  })

  it("returns the same node when the tree adds nothing", () => {
    const plain = byAddress("absent")
    expect(withLoadedFacts(plain, virtualCatalog)).toBe(plain)
    expect(withLoadedFacts(plain, null)).toBe(plain)
    const hydrated = withLoadedFacts(byAddress("chunks_vec"), virtualCatalog)
    expect(withLoadedFacts(hydrated, virtualCatalog)).toBe(hydrated)
  })
})

describe("tabsWithLoadedFacts", () => {
  it("repairs a tab opened by address once its level is read", () => {
    // A restored tab, opened before the tree held `main`.
    const tabs = [
      { key: "restored", node: byAddress("chunks_vec"), restored: "data" },
      { key: "other", node: byAddress("absent") },
    ]
    const repaired = tabsWithLoadedFacts(tabs, virtualCatalog)
    expect(repaired[0]?.node.virtualTable?.available).toBe(false)
    // Only the catalog's facts change: the tab's own state is kept.
    expect(repaired[0]?.restored).toBe("data")
    expect(repaired[1]).toBe(tabs[1])
  })

  it("keeps the same array when the tree adds nothing", () => {
    const tabs = [{ node: byAddress("absent") }]
    expect(tabsWithLoadedFacts(tabs, virtualCatalog)).toBe(tabs)
    const repaired = tabsWithLoadedFacts(
      [{ node: byAddress("chunks_vec") }],
      virtualCatalog
    )
    expect(tabsWithLoadedFacts(repaired, virtualCatalog)).toBe(repaired)
  })
})
