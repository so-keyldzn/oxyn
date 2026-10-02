import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, Download04Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

import {
  READY_BODY,
  READY_BODY_NOT_ON_QUIT,
  STALE_AFTER_DAYS,
  downloadedAgo,
  failedTitle,
} from "./update-model"
import type { PendingUpdate } from "./update-model"

/**
 * A downloaded update, or a failure the user must know of (ADR-0051,
 * docs/UX-SPEC.md « Updates »): passive, never a dialog, never repeated. The
 * host renders nothing when no update is pending.
 *
 * After seven days the trigger gains a border — a shape, not only a colour —
 * and the popover says how long ago; it insists no further. Below 1200 px the
 * label folds into the icon, with a tooltip and its accessible name: unlike
 * the status bar's other details, it is an action waiting for the user.
 */
export function UpdateIndicator({
  pending,
  compact = false,
  onRestart,
  onWhatsNew,
  onOpenSettings,
  onReleasePage,
}: {
  pending: PendingUpdate
  /** Below 1200 px of window width. */
  compact?: boolean
  onRestart: () => void
  onWhatsNew: () => void
  onOpenSettings: () => void
  onReleasePage: () => void
}) {
  const [open, setOpen] = React.useState(false)
  const laterRef = React.useRef<HTMLButtonElement>(null)
  const settingsRef = React.useRef<HTMLButtonElement>(null)
  const ready = pending.kind === "ready"
  const stale = ready && pending.days >= STALE_AFTER_DAYS
  const label = ready ? "Update ready" : "Update failed"
  const name = ready
    ? `Update ready: Oxyn ${pending.version}`
    : pending.version === null
      ? "Update failed"
      : `Update failed: Oxyn ${pending.version}`

  const trigger = (
    <Button
      variant={stale ? "outline" : "ghost"}
      size={compact ? "icon-xs" : "xs"}
      aria-label={name}
      data-stale={stale || undefined}
    />
  )
  const face = (
    <>
      <HugeiconsIcon
        icon={ready ? Download04Icon : Alert02Icon}
        strokeWidth={2}
        data-icon={compact ? undefined : "inline-start"}
        className={ready ? undefined : "text-destructive"}
        aria-hidden
      />
      {compact ? null : <span>{label}</span>}
    </>
  )

  return (
    <Popover open={open} onOpenChange={setOpen}>
      {compact ? (
        <Tooltip>
          <TooltipTrigger render={<PopoverTrigger render={trigger} />}>
            {face}
          </TooltipTrigger>
          <TooltipContent>{label}</TooltipContent>
        </Tooltip>
      ) : (
        <PopoverTrigger render={trigger}>{face}</PopoverTrigger>
      )}
      <PopoverContent
        align="end"
        side="top"
        className="w-80 max-w-[calc(100vw-2rem)]"
        initialFocus={ready ? laterRef : settingsRef}
      >
        {ready ? (
          <>
            <PopoverHeader>
              <PopoverTitle>Oxyn {pending.version} is ready</PopoverTitle>
              <PopoverDescription>
                {pending.installOnQuit ? READY_BODY : READY_BODY_NOT_ON_QUIT}
              </PopoverDescription>
              {stale ? (
                <p className="text-muted-foreground">
                  {downloadedAgo(pending.days)}
                </p>
              ) : null}
            </PopoverHeader>
            <div className="flex flex-wrap items-center gap-2">
              <Button ref={laterRef} size="sm" onClick={() => setOpen(false)}>
                Later
              </Button>
              <Button variant="outline" size="sm" onClick={onRestart}>
                Restart now
              </Button>
              <Button
                variant="link"
                size="sm"
                className="ml-auto px-0"
                onClick={onWhatsNew}
              >
                What&apos;s new
              </Button>
            </div>
          </>
        ) : (
          <>
            <PopoverHeader>
              <PopoverTitle>
                {failedTitle(pending.failure, pending.version)}
              </PopoverTitle>
            </PopoverHeader>
            <pre
              data-selectable
              dir="auto"
              className="max-h-40 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground"
            >
              {pending.message}
            </pre>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                ref={settingsRef}
                variant="outline"
                size="sm"
                onClick={() => {
                  setOpen(false)
                  onOpenSettings()
                }}
              >
                Update settings…
              </Button>
              <Button variant="outline" size="sm" onClick={onReleasePage}>
                Release page
              </Button>
            </div>
          </>
        )}
      </PopoverContent>
    </Popover>
  )
}
