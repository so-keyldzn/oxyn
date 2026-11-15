import * as React from "react"
import { metadataReads } from "@/features/metadata/read-requests"
import type { ConsoleSession, OpenConnection } from "@/lib/ipc/types"

export interface BrowsableConsole {
  key: string
  title: string
  session: ConsoleSession | null
}
interface Selection {
  connection: string
  root: string
  session: string
}
const RETIRED =
  "Console session no longer available; browsing the connection again."

/** Browsing changes the explorer's owner, never the editor or physical session. */
export function useCatalogBrowsing(
  open: OpenConnection,
  entries: Array<BrowsableConsole>,
  compatible: (owner: OpenConnection) => boolean
) {
  const [selection, setSelection] = React.useState<Selection | null>(null)
  const [notice, setNotice] = React.useState<string | null>(null)
  const live = React.useRef(true)
  const held = React.useRef(selection)
  const latest = React.useRef({ open, entries, compatible })
  latest.current = { open, entries, compatible }
  const stop = (old: Selection | null) => {
    if (old)
      void metadataReads
        .cancelCatalog(old.connection, old.session)
        .catch(() => {
          if (live.current)
            setNotice("The previous catalog read could not be cancelled.")
        })
  }
  const change = (next: Selection | null) => {
    const old = held.current
    held.current = next
    setSelection(next)
    if (old?.connection !== next?.connection || old?.session !== next?.session)
      stop(old)
  }
  const reason = (key: string): string | null => {
    const now = latest.current
    const entry = now.entries.find((candidate) => candidate.key === key)
    if (!entry?.session)
      return "Connect this console before browsing its session"
    return now.compatible({ ...now.open, ...entry.session })
      ? null
      : "This console does not provide a browsable catalog"
  }
  const browse = (key: string): boolean => {
    if (!live.current || reason(key) !== null) return false
    const now = latest.current
    const entry = now.entries.find((candidate) => candidate.key === key)
    if (!entry?.session) return false
    change({
      connection: now.open.connection,
      root: now.open.session,
      session: entry.session.session,
    })
    setNotice(null)
    return true
  }
  const entry =
    selection?.connection === open.connection && selection.root === open.session
      ? entries.find(
          (candidate) => candidate.session?.session === selection.session
        )
      : undefined
  const owner = entry?.session ? { ...open, ...entry.session } : undefined
  React.useEffect(() => {
    if (!selection || owner) return
    change(null)
    setNotice(RETIRED)
  }, [selection, owner])
  React.useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      stop(held.current)
    }
  }, [])
  return {
    owner,
    name: entry?.title || (entry ? "Untitled query" : null),
    notice,
    reason,
    browse,
    returnToConnection: () => {
      change(null)
      setNotice(null)
    },
  }
}
