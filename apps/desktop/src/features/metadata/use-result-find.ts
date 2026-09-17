import * as React from "react"

import type { FindDirection } from "@/components/oxyn/result-find-bar"
import { BackendError } from "@/lib/ipc/client"
import { MAX_PAGE_ROWS, results } from "@/lib/ipc/results"
import type { FindAnswer } from "@/lib/ipc/results"

/**
 * `Find in loaded results…` over one result, searched in Rust.
 *
 * The backend remembers the matches; this hook asks where the next one is and
 * which rows around it matched, so the grid marks them without holding fifty
 * thousand row numbers in the webview.
 */
export function useResultFind(result: string | null) {
  const [answer, setAnswer] = React.useState<FindAnswer | null>(null)
  const [needle, setNeedle] = React.useState<string | null>(null)
  const [searching, setSearching] = React.useState(false)
  const [error, setError] = React.useState<string | null>(null)
  const [reveal, setReveal] = React.useState<{
    row: number
    key: number
  } | null>(null)
  const [matches, setMatches] = React.useState<ReadonlySet<number>>(EMPTY)
  const activeRow = React.useRef(0)
  const generation = React.useRef(0)

  const clear = React.useCallback(() => {
    generation.current++
    setAnswer(null)
    setNeedle(null)
    setError(null)
    setReveal(null)
    setMatches(EMPTY)
    setSearching(false)
  }, [])

  React.useEffect(clear, [result, clear])

  const find = async (text: string, direction: FindDirection) => {
    if (!result) return
    const mine = ++generation.current
    const from =
      direction === "first"
        ? 0
        : direction === "next"
          ? activeRow.current + 1
          : activeRow.current
    setSearching(true)
    setError(null)
    setNeedle(text)
    try {
      const found = await results.findInResult(
        result,
        text,
        from,
        direction !== "previous"
      )
      if (generation.current !== mine) return
      if (found === null) {
        setAnswer(null)
        setError("This result is no longer available.")
        return
      }
      setAnswer(found)
      if (found.row !== null) {
        const row = found.row
        activeRow.current = row
        setReveal({ row, key: mine })
        const offset = Math.max(0, row - MAX_PAGE_ROWS / 2)
        const around = await results.findMatchesInWindow(
          result,
          text,
          offset,
          MAX_PAGE_ROWS
        )
        if (generation.current === mine) setMatches(new Set(around ?? []))
      } else {
        setMatches(EMPTY)
      }
    } catch (caught) {
      if (generation.current !== mine) return
      setError(caught instanceof BackendError ? caught.message : String(caught))
    } finally {
      if (generation.current === mine) setSearching(false)
    }
  }

  return {
    answer,
    needle,
    searching,
    error,
    reveal,
    matches,
    find,
    clear,
    /** The grid's active row, where « next match » starts from. */
    trackRow: (row: number) => {
      activeRow.current = row
    },
  }
}

const EMPTY: ReadonlySet<number> = new Set()
