import type { CatalogAddress, CatalogNode } from "@/lib/ipc/types"

import { addressKey } from "./catalog-tree"

/** The node the loaded tree holds at `address`, or `null`. */
export function findLoadedNode(
  nodes: ReadonlyArray<CatalogNode>,
  address: CatalogAddress
): CatalogNode | null {
  const wanted = addressKey(address)
  const pending = [...nodes]
  for (let node = pending.pop(); node; node = pending.pop()) {
    if (addressKey(node.address) === wanted) return node
    pending.push(...node.children)
  }
  return null
}

/**
 * `node` with what the loaded tree knows of the same address: its kind, its
 * virtual-table module, the virtual table it stores data for.
 *
 * An object opened by address alone — a restored tab, Quick Open, a link from
 * the assistant or from a relationship — carries none of that. Without it a
 * virtual table's view would not know its module is missing, nor where its
 * data is. The tab's own state (where it opened, what it shows) is not
 * touched; only the catalog's facts are. Returns `node` itself when the tree
 * adds nothing, so a caller can tell a change by identity.
 */
export function withLoadedFacts(
  node: CatalogNode,
  nodes: ReadonlyArray<CatalogNode> | null | undefined
): CatalogNode {
  const loaded = nodes ? findLoadedNode(nodes, node.address) : null
  if (!loaded) return node
  const facts = {
    name: loaded.name,
    kind: loaded.kind,
    holdsRecords: loaded.holdsRecords,
    system: loaded.system,
    comment: loaded.comment,
    virtualTable: loaded.virtualTable,
    shadowOf: loaded.shadowOf,
  }
  const same =
    node.name === facts.name &&
    node.kind === facts.kind &&
    node.holdsRecords === facts.holdsRecords &&
    node.system === facts.system &&
    node.comment === facts.comment &&
    node.shadowOf === facts.shadowOf &&
    JSON.stringify(node.virtualTable) === JSON.stringify(facts.virtualTable)
  return same ? node : { ...node, ...facts, loaded: true }
}

/**
 * Open object tabs completed with what the tree now knows: a tab opened by
 * address before its level was read learns, once it is, that its table is
 * virtual and that the module is missing. The same array when nothing
 * changed, so a caller renders nothing for nothing.
 */
export function tabsWithLoadedFacts<TTab extends { node: CatalogNode }>(
  tabs: Array<TTab>,
  nodes: ReadonlyArray<CatalogNode>
): Array<TTab> {
  const next = tabs.map((tab) => {
    const node = withLoadedFacts(tab.node, nodes)
    return node === tab.node ? tab : { ...tab, node }
  })
  return next.some((tab, index) => tab !== tabs[index]) ? next : tabs
}
