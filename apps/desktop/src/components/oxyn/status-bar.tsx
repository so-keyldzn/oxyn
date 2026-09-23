import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  Cancel01Icon,
  CheckmarkCircle02Icon,
  DatabaseIcon,
  InformationCircleIcon,
  LockIcon,
} from "@hugeicons/core-free-icons"
import { useThrottledValue } from "@tanstack/react-pacer"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { PrivacyTierBadge } from "@/components/oxyn/privacy-tier"
import {
  isReadOnlySession,
  surfaces,
} from "@/components/oxyn/session-capabilities"
import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover"
import { Separator } from "@/components/ui/separator"
import { Spinner } from "@/components/ui/spinner"
import type { Environment, PrivacyTier } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

export type ExecutionSummary =
  | { status: "idle" }
  | { status: "running"; rows: number }
  /** Cancel was sent; the server has not confirmed yet (UX-SPEC « Annulation »). */
  | { status: "cancelling"; rows: number }
  | { status: "done"; rows: number; elapsedMs: number }
  | { status: "failed"; message?: string; retryable?: boolean }
  | { status: "cancelled" }

/**
 * « 1 row », « 2 rows ». Grouped like every other number Oxyn writes: the
 * machine's locale would print « 1 234 » here and « 1,234 » in the grid.
 */
export function rowCount(rows: number) {
  return `${rows.toLocaleString("en-US")} ${rows === 1 ? "row" : "rows"}`
}

export function describeExecution(summary: ExecutionSummary) {
  switch (summary.status) {
    case "idle":
      return "Ready"
    case "running":
      return `Running · ${rowCount(summary.rows)}`
    case "cancelling":
      return `Cancellation requested… · ${rowCount(summary.rows)}`
    case "done":
      return `${rowCount(summary.rows)} · ${summary.elapsedMs.toLocaleString("en-US")} ms`
    case "failed":
      return summary.retryable === undefined
        ? "Failed"
        : summary.retryable
          ? "Failed · transient"
          : "Failed"
    case "cancelled":
      return "Cancelled"
  }
}

/** What a screen reader hears: the state, not every row counted. */
function announcement(summary: ExecutionSummary) {
  switch (summary.status) {
    case "running":
      return "Running"
    case "cancelling":
      return "Cancellation requested"
    default:
      return describeExecution(summary)
  }
}

/** What the session can and cannot do, said rather than emulated (ADR-0003). */
function SessionDetails({ capabilities }: { capabilities: Array<string> }) {
  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button variant="ghost" size="xs" aria-label="Session capabilities" />
        }
      >
        <HugeiconsIcon
          icon={InformationCircleIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        <span className="@max-xl:hidden">Session</span>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96 max-w-[calc(100vw-2rem)]">
        <PopoverHeader>
          <PopoverTitle>This session</PopoverTitle>
          <PopoverDescription>
            An unsupported surface is never emulated silently.
          </PopoverDescription>
        </PopoverHeader>
        <ul className="flex flex-col gap-2 text-xs">
          {surfaces(capabilities).map((surface) => (
            <li key={surface.surface} className="flex gap-2">
              <HugeiconsIcon
                icon={surface.supported ? CheckmarkCircle02Icon : Cancel01Icon}
                strokeWidth={2}
                className={cn(
                  "mt-0.5 size-3.5 shrink-0",
                  surface.supported ? "text-success" : "text-muted-foreground"
                )}
                aria-hidden
              />
              <span>
                <span className="font-medium text-foreground">
                  {surface.surface}
                  <span className="sr-only">
                    {surface.supported ? ": supported" : ": unsupported"}
                  </span>
                </span>{" "}
                <span className="text-muted-foreground">{surface.detail}</span>
              </span>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}

/**
 * Connection and execution state, always visible (docs/UX-SPEC.md).
 *
 * Only the execution status is a live region, and what it announces changes
 * at most once a second: a row counter read aloud on every batch drowns the
 * user. At reduced width the driver, the tier and the labels fold away; the
 * connection name and the execution state stay (« Largeur réduite »).
 *
 * `privacyTier` is passed only when an AI provider is declared: until then no
 * privacy badge appears anywhere (UX-SPEC « Le workspace IA n'existe que s'il
 * a été configuré »). It is the tier of **this** connection (I-04).
 */
export function StatusBar({
  connectionName,
  driver,
  environment,
  readOnly,
  execution,
  privacyTier = null,
  capabilities,
}: {
  connectionName: string
  driver: string
  environment: Environment
  readOnly: boolean
  execution: ExecutionSummary
  privacyTier?: PrivacyTier | null
  capabilities?: Array<string>
}) {
  // A session may refuse writes on its own, a replica for instance.
  const refusesWrites =
    readOnly || (capabilities !== undefined && isReadOnlySession(capabilities))
  const [spoken] = useThrottledValue(announcement(execution), {
    wait: 1000,
  })
  const failure =
    execution.status === "failed" && execution.message ? execution : null

  return (
    <div
      data-slot="status-bar"
      className="@container flex h-8 shrink-0 items-center gap-3 border-t bg-card px-3 text-xs text-muted-foreground"
    >
      <span className="flex min-w-0 items-center gap-1.5 text-foreground">
        <HugeiconsIcon
          icon={DatabaseIcon}
          strokeWidth={2}
          className="size-3.5 shrink-0"
          aria-hidden
        />
        <bdi className="truncate" title={connectionName}>
          {connectionName}
        </bdi>
      </span>
      <span className="shrink-0 @max-2xl:hidden">{driver}</span>
      <EnvironmentBadge environment={environment} className="@max-md:hidden" />
      {refusesWrites ? (
        <span className="flex shrink-0 items-center gap-1">
          <HugeiconsIcon
            icon={LockIcon}
            strokeWidth={2}
            className="size-3.5"
            aria-hidden
          />
          <span className="@max-xl:sr-only">Read only</span>
        </span>
      ) : null}
      {privacyTier ? (
        <PrivacyTierBadge tier={privacyTier} className="@max-2xl:hidden" />
      ) : null}
      <Separator orientation="vertical" className="h-4" />
      <span className="flex min-w-0 shrink-0 items-center gap-1.5 tabular-nums">
        {execution.status === "running" || execution.status === "cancelling" ? (
          <Spinner className="size-3" />
        ) : null}
        {execution.status === "done" ? (
          <HugeiconsIcon
            icon={CheckmarkCircle02Icon}
            strokeWidth={2}
            className="size-3.5 text-success"
            aria-hidden
          />
        ) : null}
        {execution.status === "failed" ? (
          <HugeiconsIcon
            icon={Alert02Icon}
            strokeWidth={2}
            className="size-3.5 text-destructive"
            aria-hidden
          />
        ) : null}
        <span aria-hidden>{describeExecution(execution)}</span>
        {failure ? (
          // A popover, not a tooltip: the server's message is meant to be
          // read at length and copied, which a tooltip that closes on the
          // first move of the pointer does not allow.
          <Popover>
            <PopoverTrigger
              render={
                <Button
                  variant="ghost"
                  size="xs"
                  className="h-5 px-1 text-foreground underline underline-offset-2"
                />
              }
            >
              Details
            </PopoverTrigger>
            <PopoverContent
              align="end"
              className="w-96 max-w-[calc(100vw-2rem)]"
            >
              <PopoverHeader>
                <PopoverTitle>What the server answered</PopoverTitle>
              </PopoverHeader>
              <pre
                data-selectable
                tabIndex={0}
                aria-label="Server message"
                className="max-h-64 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-xs whitespace-pre-wrap text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {failure.message}
              </pre>
            </PopoverContent>
          </Popover>
        ) : null}
      </span>
      <span role="status" aria-live="polite" className="sr-only">
        {spoken}
      </span>
      {capabilities ? (
        <div className="ml-auto shrink-0">
          <SessionDetails capabilities={capabilities} />
        </div>
      ) : null}
    </div>
  )
}
