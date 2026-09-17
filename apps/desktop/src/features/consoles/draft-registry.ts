import { recovery } from "@/lib/ipc/recovery"

// Drafts waiting for their typing pause, by console. The window close asks
// the webview to submit them before the backend records a clean shutdown
// (ADR-0021, ADR-0024): a draft typed in the last 250 ms is not lost.

type Flush = () => Promise<void>

const flushes = new Map<string, Flush>()

export function registerDraftFlush(key: string, flush: Flush) {
  flushes.set(key, flush)
  return () => {
    if (flushes.get(key) === flush) flushes.delete(key)
  }
}

/** Submits every pending draft now. Failures are the consoles' to report. */
export async function flushAllDrafts() {
  await Promise.allSettled([...flushes.values()].map((flush) => flush()))
}

let subscribed = false

/** Called once by the root: answers the backend's shutdown request. */
export function subscribeToShutdown() {
  if (subscribed) return
  subscribed = true
  recovery
    // One signal today, `flushDrafts`: every message asks for the same thing.
    .subscribeShutdown(() => {
      void flushAllDrafts().finally(() => {
        void recovery.shutdownFlushed().catch(() => undefined)
      })
    })
    .catch(() => {
      // Outside the desktop application there is no backend to close.
      subscribed = false
    })
}
