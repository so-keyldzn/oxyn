import type { ConsoleHandle, ConsoleMeta } from "./console-panel"
import type { ConsoleEntry } from "./use-consoles"

/** What reopening a closed console restores: text and provenance, never a run. */
export interface ClosedConsole {
  title: string
  text: string
  fromAgent: boolean
}

/**
 * Starts all confirmed closes together, then reports the committed subset.
 * Nothing unmounts while another close is still waiting for the store. Cancel
 * reaches every pending write; a close already committed cannot be undone.
 */
export async function closeConsoleBatch(
  entries: ReadonlyArray<ConsoleEntry>,
  handles: ReadonlyMap<string, ConsoleHandle>,
  meta: Readonly<Record<string, ConsoleMeta>>,
  discard: boolean,
  signal?: AbortSignal
) {
  const outcomes = await Promise.all(
    entries.map(async (entry) => {
      const handle = handles.get(entry.key)
      if (!handle || signal?.aborted || handle.closeBlocked?.()) return null
      const shown = meta[entry.key]
      const reopenable: ClosedConsole = {
        title: shown?.title ?? entry.seed.title,
        text: handle.text(),
        fromAgent: shown?.fromAgent ?? entry.seed.fromAgent,
      }
      const cancel = () => handle.cancelWrite()
      signal?.addEventListener("abort", cancel)
      try {
        return (await handle.close(discard)) ? { entry, reopenable } : null
      } catch {
        // The missing key is an explicit failed close to the batch's caller.
        return null
      } finally {
        signal?.removeEventListener("abort", cancel)
      }
    })
  )
  return outcomes.filter((outcome) => outcome !== null)
}
