import { HugeiconsIcon } from "@hugeicons/react"
import { LockIcon } from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"

/**
 * READ ONLY, drawn in the top bar and in each console of a read-only
 * connection: the refusal is announced before a statement is written, not
 * discovered when the server rejects it.
 */
export function ReadOnlyBadge() {
  return (
    <Badge variant="outline" data-slot="read-only-badge">
      <HugeiconsIcon icon={LockIcon} strokeWidth={2} data-icon="inline-start" />
      READ ONLY
    </Badge>
  )
}
