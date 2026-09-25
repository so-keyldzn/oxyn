import { HugeiconsIcon } from "@hugeicons/react"
import { AlertCircleIcon, HelpCircleIcon } from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"

/**
 * The transaction state a console session reported, beside its context
 * (ADR-0039 §5). In words: the colour alone carries nothing, and `unknown`
 * never reads as « no transaction ».
 *
 * Only drawn for `open`, and for `unknown` on a session that declares
 * `TRANSACTIONS` — see `transactionNotice`. It changes when the session
 * reports, never when a `BEGIN` or a `COMMIT` is submitted.
 */
export function TransactionBadge({ state }: { state: "open" | "unknown" }) {
  return (
    <Badge
      variant="outline"
      data-slot="transaction-badge"
      data-state={state}
      role="status"
    >
      <HugeiconsIcon
        icon={state === "open" ? AlertCircleIcon : HelpCircleIcon}
        strokeWidth={2}
        data-icon="inline-start"
      />
      {state === "open" ? "Transaction open" : "Transaction state unknown"}
    </Badge>
  )
}
