import { HugeiconsIcon } from "@hugeicons/react"
import { PlugSocketIcon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"

/**
 * The bar of a restored console that has no session yet (UX-SPEC
 * § Restauration sélective au démarrage).
 *
 * Editing, the draft and the named save work offline; running waits for the
 * user to attach the console. Attaching is its own gesture: on the connection
 * the query was written for, the same editor resumes it; on another, a copy
 * opens and the original stays here, offline. Cancelling leaves it offline.
 */
export function OfflineConsoleBar({
  connectionName,
  sameConnection,
  attaching,
  onAttach,
  onCancel,
}: {
  /** The workspace's connection, the one attaching would use. */
  connectionName: string
  /** The query was written for this connection. */
  sameConnection: boolean
  /** A session is being opened for it. */
  attaching: boolean
  onAttach: () => void
  onCancel: () => void
}) {
  return (
    <div
      data-slot="offline-console"
      className="flex min-h-9 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b bg-muted/40 px-3 py-1 text-xs"
    >
      <HugeiconsIcon
        icon={PlugSocketIcon}
        strokeWidth={2}
        className="size-3.5 shrink-0 text-muted-foreground"
      />
      <p className="min-w-0 flex-1 text-muted-foreground">
        <span className="font-medium text-foreground">Offline</span> · Choose a
        connection before running. Edits and saves stay local until then.
      </p>
      {attaching ? (
        <Button size="xs" variant="outline" onClick={onCancel}>
          <Spinner data-icon="inline-start" />
          Cancel connecting
        </Button>
      ) : (
        <Button size="xs" variant="outline" onClick={onAttach}>
          <span dir="auto" className="max-w-48 truncate">
            {sameConnection
              ? `Connect to ${connectionName}`
              : `Open a copy on ${connectionName}`}
          </span>
        </Button>
      )}
    </div>
  )
}
