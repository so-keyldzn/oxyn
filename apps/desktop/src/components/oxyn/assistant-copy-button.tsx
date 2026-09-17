import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Copy01Icon, Tick02Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

/** How long « Copied » stays before the button reads « Copy » again. */
const CONFIRMATION_MS = 2000

/**
 * Copies text the user can see, and says it did.
 *
 * `onCopy` resolves to whether the clipboard took it: a refused clipboard must
 * not show a tick.
 */
export function AssistantCopyButton({
  text,
  label,
  onCopy,
  size = "icon-xs",
}: {
  text: string
  /** What is copied, for the accessible name: « Copy answer ». */
  label: string
  onCopy: (text: string) => Promise<boolean> | boolean
  size?: "icon-xs" | "icon-sm"
}) {
  const [copied, setCopied] = React.useState(false)
  React.useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), CONFIRMATION_MS)
    return () => window.clearTimeout(timer)
  }, [copied])

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size={size}
            aria-label={copied ? `${label}: copied` : label}
            disabled={text === ""}
            onClick={async () => {
              setCopied(await onCopy(text))
            }}
          />
        }
      >
        <HugeiconsIcon
          icon={copied ? Tick02Icon : Copy01Icon}
          strokeWidth={2}
        />
      </TooltipTrigger>
      <TooltipContent>{copied ? "Copied" : label}</TooltipContent>
    </Tooltip>
  )
}
