import { holdExit, releaseExit } from "@/features/recovery/exit-transactions"
import { recovery } from "@/lib/ipc/recovery"

// Drafts waiting for their typing pause, by console. The window close asks
// the webview to submit them before the backend records a clean shutdown
// (ADR-0021, ADR-0024): a draft typed in the last 250 ms is not lost.

/** Resolves `false` when the console's text did not reach the store. */
type Flush = () => Promise<boolean>

const flushes = new Map<string, Flush>()

export function registerDraftFlush(key: string, flush: Flush) {
  flushes.set(key, flush)
  return () => {
    if (flushes.get(key) === flush) flushes.delete(key)
  }
}

/**
 * Submits every pending draft now. Resolves `true` only when every console's
 * text is in the store; each failure is its console's to report.
 */
export async function flushAllDrafts() {
  const results = await Promise.allSettled(
    [...flushes.values()].map((flush) => flush())
  )
  return results.every(
    (result) => result.status === "fulfilled" && result.value
  )
}

let subscribed = false

/** Called once by the root: answers the backend's shutdown request. */
export function subscribeToShutdown() {
  if (subscribed) return
  subscribed = true
  recovery
    .subscribeShutdown((signal) => {
      switch (signal.type) {
        case "flushDrafts":
          // A failed draft is not confirmed: the backend then says so in its
          // journal, and records the close after its grace all the same
          // (ADR-0040).
          void flushAllDrafts().then((flushed) => {
            if (flushed) void recovery.shutdownFlushed().catch(() => undefined)
          })
          return
        case "resolveTransactions":
          // Acknowledged first: past two seconds without it, the backend
          // takes this webview for frozen and exits (ADR-0043).
          void recovery.shutdownAcknowledged().catch(() => undefined)
          holdExit(signal.transactions)
          return
        case "exitCancelled":
          releaseExit()
      }
    })
    .catch(() => {
      // Outside the desktop application there is no backend to close.
      subscribed = false
    })
}
