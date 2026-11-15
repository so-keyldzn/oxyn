import * as React from "react"
import type { OpenConnection } from "@/lib/ipc/types"
import type { NativeTransferHandle } from "./native-transfer-tab"

/** Original-owner native panels are retained across connection/tab switches. */
export function useNativeTabs(activate: (key: string) => void) {
  const [entries, setEntries] = React.useState<
    Array<{ key: string; owner: OpenConnection }>
  >([])
  const handles = React.useRef(new Map<string, NativeTransferHandle>())
  const [activity, changed] = React.useReducer((value: number) => value + 1, 0)
  return {
    entries,
    handles,
    activity,
    changed,
    open: (owner: OpenConnection) => {
      const key = `native:${JSON.stringify([owner.connection, owner.session])}`
      setEntries((previous) =>
        previous.some((entry) => entry.key === key)
          ? previous
          : [...previous, { key, owner }]
      )
      activate(key)
    },
    close: (keys: ReadonlySet<string>) =>
      setEntries((previous) =>
        previous.filter((entry) => !keys.has(entry.key))
      ),
    retainOwners: (owners: ReadonlySet<string>) =>
      setEntries((previous) =>
        previous.filter((entry) => owners.has(entry.owner.session))
      ),
  }
}
