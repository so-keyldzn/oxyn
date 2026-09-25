import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, TimeQuarterPassIcon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"

/**
 * What an object tab brought back from the last session says above its views
 * (docs/UX-SPEC.md, « Restauration après un arrêt brutal »).
 *
 * `held`: the place came back and nothing was read — not the rows, not the
 * metadata — until the user asks. `vanished`: read again, the object was not
 * there. Its place is explained and kept, never erased on the user's behalf:
 * closing the tab is what forgets it.
 */
export function RestoredObjectNotice({
  state,
  name,
  onRead,
}: {
  state: "held" | "vanished"
  /** Text, never markup: an object name is an input like any other. */
  name: string
  /** `Read it now`, while held. */
  onRead?: () => void
}) {
  if (state === "vanished") {
    return (
      <Alert className="rounded-none border-x-0 border-t-0 py-2">
        <HugeiconsIcon
          icon={Alert02Icon}
          strokeWidth={2}
          className="text-warning"
        />
        <AlertTitle>
          <span dir="auto">{name}</span> was not found
        </AlertTitle>
        <AlertDescription>
          It was open when Oxyn last closed. Read again, the catalog no longer
          holds it: it may have been renamed or dropped. Its place is kept until
          you close this tab.
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <Alert className="rounded-none border-x-0 border-t-0 py-2">
      <HugeiconsIcon icon={TimeQuarterPassIcon} strokeWidth={2} />
      <AlertTitle>Restored from your last session</AlertTitle>
      <AlertDescription className="flex flex-wrap items-center gap-x-3 gap-y-1">
        Nothing has been read from the server yet.
        {onRead ? (
          <Button size="xs" variant="outline" onClick={onRead}>
            Read it now
          </Button>
        ) : null}
      </AlertDescription>
    </Alert>
  )
}
