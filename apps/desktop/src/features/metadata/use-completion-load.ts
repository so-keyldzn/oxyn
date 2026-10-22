import * as React from "react"

import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { completion } from "@/lib/ipc/completion"
import type { NamePart } from "@/lib/sql-identifiers"

type Names = Array<Array<NamePart>>

/**
 * Has the host read what the catalog cache lacks for the statement under the
 * cursor (ADR-0057 § 2), and returns the function the editor calls with that
 * statement's names.
 *
 * The editor calls it after a pause in typing; this does not wait any more. It
 * sends a statement's names once: the same names again cost nothing, since the
 * host already read what they need. It sends them again when
 *
 * - a DDL invalidated the cache (`catalogInvalidated`): what was read is to be
 *   read again, and the editor has nothing new to say;
 * - a pass ended before it was done — out of time, or replaced by another
 *   console's of the connection: what is left is still unread, and the next
 *   call continues with it.
 *
 * **Only the console on screen sends.** Consoles stay mounted behind the
 * active one, and a host holds one pass per connection: hidden consoles would
 * replace each other's, and read a server for tabs nobody looks at. A console
 * keeps the names it was last given, and sends them when it comes forward.
 *
 * What a pass reads reaches the completion through the refresh signals
 * (`useCompletionCatalog`), not through this: the answer is not awaited by
 * anything, and a pass that fails costs the completion only the names it did
 * not get.
 */
export function useCompletionLoad(
  connection: string,
  active: boolean
): (names: Names) => void {
  // The last names the editor gave, and those a pass was sent for.
  const latest = React.useRef<Names | null>(null)
  const sent = React.useRef<string | null>(null)
  const shown = React.useRef(active)
  shown.current = active

  // Another connection has its own cache: nothing sent for this one counts.
  React.useEffect(() => {
    latest.current = null
    sent.current = null
  }, [connection])

  const send = React.useCallback(
    (names: Names) => {
      const key = JSON.stringify(names)
      if (key === sent.current) return
      sent.current = key
      completion
        .load(connection, names)
        .then((load) => {
          // Unfinished: out of time, or replaced by a pass of another console
          // of the connection. Unless this one sent newer names since — those
          // took its place on the host — the next call continues.
          if (load.stopped !== null && sent.current === key) sent.current = null
        })
        // Best effort: the completion goes on with what the cache holds, and
        // the next call tries again.
        .catch(() => {
          if (sent.current === key) sent.current = null
        })
    },
    [connection]
  )

  // Coming forward: what the editor said while hidden is still to be read.
  React.useEffect(() => {
    if (active && latest.current !== null) send(latest.current)
  }, [active, send])

  useRefreshSignal(connection, (signal) => {
    if (signal.type !== "catalogInvalidated") return
    sent.current = null
    if (shown.current && latest.current !== null) send(latest.current)
  })

  return React.useCallback(
    (names: Names) => {
      latest.current = names
      if (shown.current) send(names)
    },
    [send]
  )
}
