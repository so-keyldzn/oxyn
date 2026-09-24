import * as React from "react"
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
import { toolCardOf } from "@/features/assistant/transcript"
import type {
  RestoredCallEntry,
  ToolCallEntry,
  ToolCallState,
  ToolCard,
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
 * The card of one tool call, live or reopened: the same header, the same
 * statement block, the same report. Only what the card is given differs.
 */
function ToolCallCard({
  card,
  footer,
  rows,
}: {
  card: ToolCard
  /** Under the report, inside its section: the approval that waits. */
  footer?: React.ReactNode
  rows?: React.ReactNode
}) {
  const state = STATES[card.state]
  return (
    <section
      data-slot="assistant-tool-call"
      data-state={card.state}
      aria-label={
        card.target
          ? `Command ${card.tool} on ${card.target.connection}: ${state.label}`
          : `Command ${card.tool}: ${state.label}`
      }
      className={cn(
        "flex min-w-0 flex-col overflow-hidden rounded-lg border bg-card text-sm",
        card.state === "awaitingApproval" && "border-warning/60"
      )}
    >
      <header className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b px-3 py-2">
        <HugeiconsIcon
          icon={CommandLineIcon}
          strokeWidth={2}
          className="size-4 text-muted-foreground"
          aria-hidden
        />
        <span className="font-mono text-xs">{card.tool}</span>
        {card.target ? (
          <>
            <span className="text-xs text-muted-foreground">on</span>
            <span className="truncate text-xs font-medium">
              {card.target.connection}
            </span>
            {card.target.environment ? (
              <EnvironmentBadge environment={card.target.environment} />
            ) : null}
          </>
        ) : null}
        {card.mutating !== null ? (
          <span
            className={cn(
              "text-xs",
              card.mutating ? "text-warning" : "text-muted-foreground"
            )}
          >
            {card.mutating ? "May change data" : "Read only"}
          </span>
        ) : null}
        <Badge
          variant={state.tone === "danger" ? "destructive" : "outline"}
          className={cn(
            "ml-auto",
            state.tone === "warning" && "border-warning text-warning",
            state.tone === "success" && "border-success text-success"
          )}
        >
          <StateIcon state={card.state} />
          {state.label}
        </Badge>
      </header>

      {card.statement ? (
        // Wrapped at spaces only: a word of a statement is never cut in two,
        // and a longer token scrolls sideways within the block.
        <pre
          data-selectable
          tabIndex={0}
          aria-label="Statement the agent submitted"
          className="max-h-48 overflow-auto px-3 py-2 font-mono text-xs leading-5 whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {card.statement}
        </pre>
      ) : null}

      {card.detail || card.withheld || footer ? (
        <div className="flex flex-col gap-2 border-t px-3 py-2">
          {card.detail ? (
            <p
              data-selectable
              className={cn(
                "font-mono text-xs break-words whitespace-pre-wrap",
                (card.state === "failed" || card.state === "denied") &&
                  "text-destructive"
              )}
            >
              {card.errorClass ? (
                <span className="mr-1 font-sans font-medium">
                  {CLASSES[card.errorClass]} —
                </span>
              ) : null}
              {card.detail}
            </p>
          ) : null}
          {card.withheld ? (
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
          {footer}
        </div>
      ) : null}
      {rows}
    </section>
  )
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
  rows,
}: {
  entry: ToolCallEntry
  onReview?: (entry: ToolCallEntry) => void
  /** The rows the call returned, drawn under its report (`AssistantToolRows`). */
  rows?: React.ReactNode
}) {
  return (
    <ToolCallCard
      card={toolCardOf(entry)}
      rows={rows}
      footer={
        entry.approval &&
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
        ) : null
      }
    />
  )
}

/**
 * A tool call of a conversation reopened from the workspace, drawn by the
 * same card as live. Its rows are gone with the process that read them: the
 * card says so where the grid was (docs/UX-SPEC.md), and nothing is rerun.
 */
export function AssistantRestoredCall({ entry }: { entry: RestoredCallEntry }) {
  return (
    <ToolCallCard
      card={toolCardOf(entry)}
      rows={
        entry.rowsNotKept ? (
          <p
            data-slot="assistant-tool-rows-not-kept"
            className="border-t px-3 py-2 text-xs text-muted-foreground"
          >
            Result no longer available: the workspace keeps the statement, never
            its rows. Nothing is rerun.
          </p>
        ) : null
      }
    />
  )
}
