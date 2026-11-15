import * as React from "react"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"
import type { MetadataScope } from "@/lib/ipc/metadata-loading"

interface MapTab {
  key: string
  owner: OpenConnection
  scope: MetadataScope | null
  object?: CatalogNode
}

/** Map tabs retain their original source; changing the explorer never retargets them. */
export function useMapTabs(activate: (key: string) => void) {
  const [entries, setEntries] = React.useState<Array<MapTab>>([])
  return {
    entries,
    open: (owner: OpenConnection, node?: CatalogNode) => {
      const scope = node
        ? { catalog: node.address.catalog, namespace: node.address.namespace }
        : null
      const key = `map:${JSON.stringify([owner.connection, owner.session, node?.address ?? null])}`
      setEntries((all) =>
        all.some((entry) => entry.key === key)
          ? all
          : [
              ...all,
              {
                key,
                owner,
                scope,
                object: node?.address.relation ? node : undefined,
              },
            ]
      )
      activate(key)
    },
    close: (keys: ReadonlySet<string>) =>
      setEntries((all) => all.filter((entry) => !keys.has(entry.key))),
    retainOwners: (owners: ReadonlySet<string>) =>
      setEntries((all) =>
        all.filter((entry) => owners.has(entry.owner.session))
      ),
  }
}
