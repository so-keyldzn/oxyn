import * as React from "react"

import { needsCloseDecision } from "@/features/consoles/console-model"
import type { CloseReasons } from "@/features/consoles/console-model"

export interface TabCloseTarget {
  key: string
  title: string
  reasons: CloseReasons | null
  blocked: string | null
}

export interface TabCloseReview {
  tabs: ReadonlyArray<TabCloseTarget>
  busy: boolean
  notice: string | null
}

/** One decision for a snapshot of tabs; tabs opened afterwards are untouched. */
export function useCloseTabs(effects: {
  targets: () => ReadonlyArray<TabCloseTarget>
  save: (key: string) => Promise<boolean>
  cancelWrite: (key: string) => void
  close: (
    keys: ReadonlyArray<string>,
    discard: boolean,
    signal: AbortSignal
  ) => Promise<ReadonlyArray<string>>
}) {
  const latest = React.useRef(effects)
  latest.current = effects
  const [review, setReview] = React.useState<TabCloseReview | null>(null)
  const current = React.useRef<TabCloseReview | null>(null)
  const request = React.useRef<AbortController | null>(null)
  const publish = (next: TabCloseReview | null) => {
    current.current = next
    setReview(next)
  }

  const cancel = () => {
    request.current?.abort()
    request.current = null
    publish(null)
  }
  React.useEffect(() => () => request.current?.abort(), [])

  const decide = async (choice: "save" | "discard") => {
    const held = current.current
    if (!held || held.busy) return
    const controller = new AbortController()
    request.current = controller
    const { signal } = controller
    // Read afresh after each await; TypeScript otherwise retains its earlier
    // narrowing of AbortSignal's readonly property across asynchronous work.
    const cancelled = () => signal.aborted
    const keys = held.tabs.map((tab) => tab.key)
    const cancelWrites = () =>
      keys.forEach((key) => latest.current.cancelWrite(key))
    signal.addEventListener("abort", cancelWrites)
    publish({ ...held, busy: true, notice: null })
    try {
      // Recheck activity before touching any document. A running save or
      // export is never discarded just because the batch was requested.
      const targets = latest.current
        .targets()
        .filter((tab) => keys.includes(tab.key))
      const blocked = targets.filter((tab) => tab.blocked)
      if (blocked.length > 0) {
        publish({
          ...held,
          busy: false,
          notice: blocked
            .map((tab) => `${tab.title}: ${tab.blocked}`)
            .join("\n"),
        })
        return
      }
      if (choice === "save") {
        const saving = targets.filter(
          (tab) => tab.reasons?.unsaved || tab.reasons?.conflict
        )
        const saved = await Promise.allSettled(
          saving.map((tab) => latest.current.save(tab.key))
        )
        if (cancelled()) return
        const failed = saving.filter((_, index) => {
          const outcome = saved[index]
          return outcome?.status !== "fulfilled" || !outcome.value
        })
        if (failed.length > 0) {
          publish({
            ...held,
            busy: false,
            notice: `Not saved: ${failed.map((tab) => tab.title).join(", ")}. No tabs were closed. Check their save status before trying again.`,
          })
          return
        }
      }
      if (cancelled()) return
      signal.removeEventListener("abort", cancelWrites)
      const closed = await latest.current.close(
        keys,
        choice === "discard",
        signal
      )
      if (cancelled()) return
      const remaining = held.tabs.filter((tab) => !closed.includes(tab.key))
      publish(
        remaining.length === 0
          ? null
          : {
              tabs: remaining,
              busy: false,
              notice:
                "These tabs could not be closed and remain open. Check their save status before trying again.",
            }
      )
    } catch (error) {
      if (!cancelled())
        publish({
          ...held,
          busy: false,
          notice: error instanceof Error ? error.message : String(error),
        })
    } finally {
      signal.removeEventListener("abort", cancelWrites)
      if (request.current === controller) request.current = null
    }
  }

  return {
    review,
    cancel,
    decide,
    request: (keys: ReadonlyArray<string>) => {
      if (current.current !== null) return
      const tabs = latest.current
        .targets()
        .filter((tab) => keys.includes(tab.key))
      if (tabs.length === 0) return
      publish({ tabs, busy: false, notice: null })
      if (
        tabs.every(
          (tab) =>
            !tab.blocked && (!tab.reasons || !needsCloseDecision(tab.reasons))
        )
      )
        void decide("save")
    },
  }
}
