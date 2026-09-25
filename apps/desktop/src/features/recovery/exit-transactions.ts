import { createStore } from "@tanstack/react-store"

import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { consoles } from "@/lib/ipc/consoles"
import { recovery } from "@/lib/ipc/recovery"
import type { ExitTransaction } from "@/lib/ipc/recovery"
import { results } from "@/lib/ipc/results"
import type { TransactionState } from "@/lib/ipc/types"

// The exit held by an open transaction (ADR-0043). The backend lists the
// transactions and says when to ask; the dialog answers with `COMMIT` or
// `ROLLBACK` through `run_console` — the path of a statement typed in the
// console, gate and history included (I-01) — then asks for the exit again,
// and the backend reads the states once more.

export type ExitDecision = "commit" | "rollback"

export interface ExitHold {
  /** As the backend listed them, last signal first to last. */
  transactions: Array<ExitTransaction>
  busy: ExitDecision | null
  /** The server's message of the last failure, as it came. */
  error: string | null
  /**
   * Sessions whose `COMMIT` failed or ended without an answer. Never sent
   * again from this dialog (I-13): the state the session reports afterwards
   * says whether it took, and the user may still roll back or cancel.
   */
  commitFailed: Array<string>
  /**
   * A statement ran since the backend listed these: the states shown are
   * then the ones the sessions reported since, not the listing's.
   */
  ran: boolean
}

/** `null` while no exit is held. */
export const exitHold = createStore<ExitHold | null>(null)

/** A listed transaction, with the state to show and the console's name. */
export interface ShownTransaction extends ExitTransaction {
  console: string
}

/**
 * What the dialog shows: the listing's states until a statement runs, then
 * what each session reported since. A session back to `idle` leaves the list.
 */
export function shownTransactions(
  hold: ExitHold,
  labels: Record<string, string>,
  live: Record<string, TransactionState>
): Array<ShownTransaction> {
  return hold.transactions
    .map((transaction) => ({
      ...transaction,
      console: labels[transaction.session] ?? "Console",
      state: hold.ran
        ? (live[transaction.session] ?? transaction.state)
        : transaction.state,
    }))
    .filter((transaction) => transaction.state !== "idle")
}

/** Whether no listed session may still receive a `COMMIT` from here. */
export function commitBlocked(
  hold: ExitHold,
  shown: ReadonlyArray<ShownTransaction>
) {
  return (
    shown.length > 0 &&
    shown.every((transaction) =>
      hold.commitFailed.includes(transaction.session)
    )
  )
}

/** `resolveTransactions`: the backend holds the exit on these sessions. */
export function holdExit(transactions: Array<ExitTransaction>) {
  exitHold.setState((current) => ({
    transactions,
    busy: null,
    error: current?.error ?? null,
    commitFailed: current?.commitFailed ?? [],
    ran: false,
  }))
}

/** `exitCancelled`, or this dialog's own Cancel. */
export function releaseExit() {
  exitHold.setState(() => null)
}

export function cancelExit() {
  releaseExit()
  void recovery.cancelExit().catch(() => undefined)
}

const STATEMENT: Record<ExitDecision, string> = {
  commit: "COMMIT",
  rollback: "ROLLBACK",
}

/**
 * Sends `COMMIT` or `ROLLBACK` to each listed session in turn, and stops at
 * the first failure, which the dialog shows: the exit does not go on. When
 * every statement ran, asks for the exit again: the backend reads the states
 * and either shuts down or lists what is still open.
 *
 * `pending` is what the dialog still shows open or unknown. A `COMMIT` is not
 * sent to a session whose `COMMIT` already failed here.
 */
export async function resolveExit(
  decision: ExitDecision,
  pending: Array<ExitTransaction>
) {
  const hold = exitHold.state
  if (!hold || hold.busy) return
  const targets = pending.filter(
    (transaction) =>
      decision === "rollback" ||
      !hold.commitFailed.includes(transaction.session)
  )
  update({ busy: decision, error: null, ran: true })
  for (const transaction of targets) {
    const failure = await send(decision, transaction)
    if (failure !== null) {
      exitHold.setState((current) =>
        current
          ? {
              ...current,
              busy: null,
              error: failure,
              commitFailed:
                decision === "commit"
                  ? [...current.commitFailed, transaction.session]
                  : current.commitFailed,
            }
          : current
      )
      return
    }
  }
  update({ busy: null })
  // Cancelled meanwhile: the exit is no longer asked for.
  if (exitHold.state === null) return
  await recovery.requestExit().catch((error: unknown) => {
    update({ error: message(error) })
  })
}

/** The failure to show, or `null` when the statement ran. */
async function send(decision: ExitDecision, transaction: ExitTransaction) {
  try {
    const outcome = await consoles.run(
      newCommandId(),
      transaction.connection,
      transaction.session,
      {
        sql: STATEMENT[decision],
        target: { kind: "all" },
        parameters: [],
        explain: false,
      }
    )
    switch (outcome.type) {
      case "executed":
        // Nothing to read in it: released at once.
        void results.forgetResult(outcome.result).catch(() => undefined)
        return outcome.cancelled
          ? `${STATEMENT[decision]} was cancelled; its outcome is not known.`
          : null
      case "denied":
        return outcome.reason
      case "needsApproval":
        // Not approved from here: the dialog decides on the transaction, not
        // on a statement the gate holds. Rejected, so nothing waits on it.
        void backend.decide(outcome.command, false).catch(() => undefined)
        return `${STATEMENT[decision]} needs an approval: ${outcome.reason}`
      case "cancelled":
        return `${STATEMENT[decision]} was cancelled; its outcome is not known.`
      default:
        return `${STATEMENT[decision]} did not run: ${outcome.type}.`
    }
  } catch (error) {
    return message(error)
  }
}

function message(error: unknown) {
  return error instanceof BackendError ? error.message : String(error)
}

function update(patch: Partial<ExitHold>) {
  exitHold.setState((current) => (current ? { ...current, ...patch } : current))
}
