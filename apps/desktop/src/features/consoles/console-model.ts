// What a console shows about itself, as pure functions: the tab label, why a
// close needs a decision, what the save line says. Kept apart from React so
// the rules are tested without a screen.

import type { TransactionState } from "@/lib/ipc/types"

/** What a console is busy with, by priority on its tab. */
export type ConsoleActivity =
  "idle" | "running" | "approval" | "saving" | "closing"

export interface ConsoleTabInfo {
  key: string
  title: string
  fromAgent: boolean
  activity: ConsoleActivity
  unsaved: boolean
}

/** The naming: `console_1.sql`, a counter that never goes back. */
export function consoleTitle(counter: number) {
  return `console_${counter}.sql`
}

export function tabTitle(info: Pick<ConsoleTabInfo, "title" | "fromAgent">) {
  const title = info.title.trim() === "" ? "Untitled query" : info.title
  return info.fromAgent ? `AI · ${title}` : title
}

/** `closing` > `saving` > `approval` > `running` > `unsaved`. */
export function tabState(info: Pick<ConsoleTabInfo, "activity" | "unsaved">) {
  if (info.activity !== "idle") return info.activity
  return info.unsaved ? "unsaved" : null
}

/**
 * What the console bar says about its session's transaction (ADR-0039 §5):
 * `open`, `unknown` on a session that declares `TRANSACTIONS`, and nothing
 * otherwise — no dimmed marker suggesting a transaction where none can be.
 * A session that declares it but reported nothing yet is `unknown`, never
 * `idle`.
 */
export function transactionNotice(
  state: TransactionState | undefined,
  declaresTransactions: boolean
): "open" | "unknown" | null {
  if (!declaresTransactions) return null
  if (state === "idle") return null
  return state === "open" ? "open" : "unknown"
}

export interface CloseReasons {
  title: string
  /** The connection the console's session belongs to, named by the dialog. */
  connection: string
  /** The stored document moved elsewhere. */
  conflict: boolean
  hasSavedCopy: boolean
  unsaved: boolean
  running: boolean
  /** What `transactionNotice` says: closing rolls back what is open. */
  transaction: "open" | "unknown" | null
}

/** Closing immediately is safe only when nothing would be lost or stopped. */
export function needsCloseDecision(reasons: CloseReasons) {
  return (
    reasons.conflict ||
    reasons.unsaved ||
    reasons.running ||
    reasons.transaction !== null
  )
}

/** The dialog text, composed from what closing would do (ADR-0015). */
export function closeMessage(reasons: CloseReasons) {
  const parts: Array<string> = []
  // First: of everything a close can lose, uncommitted writes are what no
  // saved copy brings back (ADR-0039 §5).
  if (reasons.transaction === "open")
    parts.push(
      `A transaction is open on ${reasons.connection}. Closing this console rolls it back: its uncommitted changes will be lost.`
    )
  if (reasons.transaction === "unknown")
    parts.push(
      `The transaction state on ${reasons.connection} is unknown. If a transaction is open, closing this console rolls it back: its uncommitted changes will be lost.`
    )
  if (reasons.conflict)
    parts.push(
      "The stored document changed elsewhere and will be left untouched."
    )
  if (reasons.hasSavedCopy)
    parts.push("The saved copy will remain in the query library.")
  if (reasons.unsaved)
    parts.push(
      "The SQL in this console has not been saved. Closing discards this text."
    )
  if (reasons.running)
    parts.push(
      "The running statement will be cancelled. Closing does not undo committed database changes."
    )
  return parts.join(" ")
}

/** The resting save line, when no save is under way. */
export function saveNotice({
  hasSavedCopy,
  unsaved,
}: {
  hasSavedCopy: boolean
  unsaved: boolean
}) {
  if (!hasSavedCopy) return "No saved copy yet."
  return unsaved
    ? "Unsaved changes since the last save."
    : "Matches the saved version."
}

/** The next console in tab order, wrapping; `null` with fewer than two. */
export function cycleConsole(
  keys: ReadonlyArray<string>,
  active: string | null,
  delta: 1 | -1
) {
  if (keys.length < 2) return null
  const index = active === null ? -1 : keys.indexOf(active)
  const next = (index + delta + keys.length) % keys.length
  return keys[next] ?? null
}

/** Query names are bounded by the store: 256 UTF-8 bytes. */
export function titleTooLong(title: string) {
  return new TextEncoder().encode(title).length > 256
}
