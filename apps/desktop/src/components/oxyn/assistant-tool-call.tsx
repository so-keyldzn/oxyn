import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  CancelCircleIcon,
  CheckmarkCircle02Icon,
  Clock01Icon,
  CommandLineIcon,
  StopCircleIcon,
} from "@hugeicons/core-free-icons"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import type {
  ToolCallEntry,
  ToolCallState,
} from "@/features/assistant/transcript"
import type { ErrorClass } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

const STATES: Record<
  ToolCallState,
  { label: string; tone: "muted" | "warning" | "danger" | "success" }
> = {
  running: { label: "Running", tone: "muted" },
  awaitingApproval: { label: "Waiting for your approval", tone: "warning" },
  deciding: { label: "Deciding…", tone: "muted" },
  completed: { label: "Completed", tone: "success" },
  denied: { label: "Refused · nothing ran", tone: "danger" },
  rejected: { label: "Rejected by you · nothing ran", tone: "muted" },
  failed: { label: "Failed", tone: "danger" },
  cancelled: { label: "Stopped", tone: "muted" },
}

const CLASSES: Record<ErrorClass, string> = {
  transient: "transient",
  permanent: "permanent",
  // I-13: the only family that must say it may have applied.
  ambiguous: "ambiguous · it may have applied",
  unknown: "unclassified",
}

function StateIcon({ state }: { state: ToolCallState }) {
  switch (state) {
    case "running":
    case "deciding":
      // The label beside says the state; a spinner's own `status` would be
      // one more live region in a panel that announces once.
      return <Spinner data-icon="inline-start" aria-hidden />
    case "awaitingApproval":
      return (
        <HugeiconsIcon
          icon={Clock01Icon}
          strokeWidth={2}
          data-icon="inline-start"
        />
      )
    case "completed":
      return (
        <HugeiconsIcon
          icon={CheckmarkCircle02Icon}
          strokeWidth={2}
          data-icon="inline-start"
        />
      )
    case "cancelled":
    case "rejected":
      return (
        <HugeiconsIcon
          icon={StopCircleIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
      )
    case "denied":
    case "failed":
      return (
        <HugeiconsIcon
          icon={CancelCircleIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
      )
  }
}

/**
 * A command the agent submitted, shown before its result (docs/UX-SPEC.md).
 *
 * The exact statement, the connection by **name** with its environment, and
 * whether it may change data are shown from the moment it leaves. An approval
 * is never given here: `Review…` opens the review that names the connection.
 */
export function AssistantToolCall({
  entry,
  onReview,
}: {
  entry: ToolCallEntry
  onReview?: (entry: ToolCallEntry) => void
}) {
  const state = STATES[entry.state]
  return (
    <section
      data-slot="assistant-tool-call"
      data-state={entry.state}
      aria-label={`Command ${entry.tool} on ${entry.connection}: ${state.label}`}
      className={cn(
        "flex min-w-0 flex-col overflow-hidden rounded-lg border bg-card text-sm",
        entry.state === "awaitingApproval" && "border-warning/60"
      )}
    >
      <header className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b px-3 py-2">
        <HugeiconsIcon
          icon={CommandLineIcon}
          strokeWidth={2}
          className="size-4 text-muted-foreground"
          aria-hidden
        />
        <span className="font-mono text-xs">{entry.tool}</span>
        <span className="text-xs text-muted-foreground">on</span>
        <span className="truncate text-xs font-medium">{entry.connection}</span>
        {entry.environment ? (
          <EnvironmentBadge environment={entry.environment} />
        ) : null}
        <span
          className={cn(
            "text-xs",
            entry.mutating ? "text-warning" : "text-muted-foreground"
          )}
        >
          {entry.mutating ? "May change data" : "Read only"}
        </span>
        <Badge
          variant={state.tone === "danger" ? "destructive" : "outline"}
          className={cn(
            "ml-auto",
            state.tone === "warning" && "border-warning text-warning",
            state.tone === "success" && "border-success text-success"
          )}
        >
          <StateIcon state={entry.state} />
          {state.label}
        </Badge>
      </header>

      {entry.statement ? (
        <pre
          data-selectable
          tabIndex={0}
          aria-label="Statement the agent submitted"
          className="max-h-48 overflow-auto px-3 py-2 font-mono text-xs leading-5 whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {entry.statement}
        </pre>
      ) : null}

      {entry.detail || entry.withheld || entry.approval ? (
        <div className="flex flex-col gap-2 border-t px-3 py-2">
          {entry.detail ? (
            <p
              data-selectable
              className={cn(
                "font-mono text-xs break-words whitespace-pre-wrap",
                (entry.state === "failed" || entry.state === "denied") &&
                  "text-destructive"
              )}
            >
              {entry.errorClass ? (
                <span className="mr-1 font-sans font-medium">
                  {CLASSES[entry.errorClass]} —
                </span>
              ) : null}
              {entry.detail}
            </p>
          ) : null}
          {entry.withheld ? (
            <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
              <HugeiconsIcon
                icon={Alert02Icon}
                strokeWidth={2}
                className="mt-0.5 size-3.5 shrink-0"
                aria-hidden
              />
              The model received less than this: the connection's privacy tier
              withheld part of it.
            </p>
          ) : null}
          {entry.approval &&
          (entry.state === "awaitingApproval" || entry.state === "deciding") ? (
            <div className="flex flex-wrap items-center gap-2">
              <p className="min-w-0 flex-1 text-xs text-muted-foreground">
                Nothing ran. The agent asks to run this on{" "}
                <strong className="font-medium text-foreground">
                  {entry.approval.connection}
                </strong>
                .
              </p>
              <Button
                size="sm"
                variant="outline"
                disabled={entry.state === "deciding" || onReview === undefined}
                onClick={() => onReview?.(entry)}
              >
                {entry.state === "deciding" ? (
                  <Spinner data-icon="inline-start" />
                ) : null}
                Review…
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}
