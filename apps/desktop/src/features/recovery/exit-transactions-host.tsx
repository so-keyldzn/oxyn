import { useStore } from "@tanstack/react-store"

import { ExitTransactionsDialog } from "@/components/oxyn/exit-transactions-dialog"
import { consoleLabels } from "@/features/consoles/console-labels"
import {
  cancelExit,
  commitBlocked,
  exitHold,
  resolveExit,
  shownTransactions,
} from "@/features/recovery/exit-transactions"
import { updateStore } from "@/features/updates/update-store"
import { transactionStates } from "@/lib/ipc/events"

/**
 * The dialog of an exit, or of this window's close, held by an open
 * transaction: once per window, on its own consoles (ADR-0043).
 */
export function ExitTransactionsHost() {
  const hold = useStore(exitHold)
  const labels = useStore(consoleLabels)
  const live = useStore(transactionStates)
  const shown = hold ? shownTransactions(hold, labels, live) : null
  // What a `restart` installs (ADR-0051).
  const update = useStore(updateStore, (view) => view.snapshot?.state)
  return (
    <ExitTransactionsDialog
      rows={shown}
      scope={hold?.scope ?? "application"}
      version={update?.type === "ready" ? update.version : null}
      busy={hold?.busy ?? null}
      error={hold?.error ?? null}
      commitBlocked={hold && shown ? commitBlocked(hold, shown) : false}
      onCommit={() => void resolveExit("commit", shown ?? [])}
      onRollback={() => void resolveExit("rollback", shown ?? [])}
      onCancel={cancelExit}
    />
  )
}
