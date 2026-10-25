import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  CancelCircleIcon,
  CheckmarkCircle02Icon,
  Clock01Icon,
  InformationCircleIcon,
  SquareLock02Icon,
} from "@hugeicons/core-free-icons"

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { Marker, MarkerContent, MarkerIcon } from "@/components/ui/marker"
import { Spinner } from "@/components/ui/spinner"
import {
  MEMORY_RESET_LINES,
  catalogLine,
} from "@/features/assistant/transcript"
import type { CatalogEntry } from "@/features/assistant/transcript"
import type { AgentToolStatus, MemoryReset } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

const TOOL_STATUS: Record<AgentToolStatus, string> = {
  pending: "waiting",
  running: "working",
  completed: "done",
  failed: "failed",
}

function StepIcon({ status }: { status: AgentToolStatus }) {
  if (status === "running") return <Spinner aria-hidden role={undefined} />
  return (
    <HugeiconsIcon
      icon={
        status === "pending"
          ? Clock01Icon
          : status === "completed"
            ? CheckmarkCircle02Icon
            : CancelCircleIcon
      }
      strokeWidth={2}
      aria-hidden
      className={cn(status === "failed" && "text-destructive")}
    />
  )
}

/** « Agent step · read · done »: the words carry the state, not the icon. */
function StepText({ kind, status }: { kind: string; status: AgentToolStatus }) {
  return (
    <>
      <span>Agent step · </span>
      <span className="font-medium text-foreground">{kind}</span>
      <span className={cn(status === "failed" && "text-destructive")}>
        {` · ${TOOL_STATUS[status]}`}
      </span>
    </>
  )
}

/**
 * A step of an external agent's own work — reading, searching, thinking.
 *
 * It is not an Oxyn command: nothing went through the bus. Only the kind is
 * shown, never the agent's title for it, which can quote a path of the machine.
 */
export function AssistantAgentTool({
  tool,
  status,
}: {
  tool: string | null
  status: AgentToolStatus
}) {
  return (
    <Marker
      data-slot="assistant-agent-tool"
      data-status={status}
      className="text-xs"
    >
      <MarkerIcon>
        <StepIcon status={status} />
      </MarkerIcon>
      <MarkerContent>
        <StepText kind={tool ?? "tool"} status={status} />
      </MarkerContent>
    </Marker>
  )
}

export interface AgentStep {
  key: string
  tool: string | null
  status: AgentToolStatus
}

function counted(count: number, noun: string) {
  return `${count} ${noun}${count === 1 ? "" : "s"}`
}

/**
 * Consecutive steps of an external agent, folded under one line: how many,
 * the one in progress by its kind, and how many failed — said in words, so
 * a failure is never only a red icon in a closed group. One step alone is
 * drawn as itself.
 *
 * Only `agentTool` entries are folded here. What Oxyn refused stays a line of
 * its own between them, never inside a group that could be closed.
 */
export function AssistantAgentSteps({
  steps,
}: {
  steps: ReadonlyArray<AgentStep>
}) {
  const [open, setOpen] = React.useState(false)
  const first = steps[0]
  if (!first) return null
  if (steps.length === 1)
    return <AssistantAgentTool tool={first.tool} status={first.status} />

  const live = [...steps]
    .reverse()
    .find((step) => step.status === "running" || step.status === "pending")
  const failed = steps.filter((step) => step.status === "failed").length
  const summaryStatus: AgentToolStatus = live
    ? live.status
    : failed > 0
      ? "failed"
      : "completed"

  return (
    <Collapsible
      data-slot="assistant-agent-steps"
      data-status={summaryStatus}
      open={open}
      onOpenChange={setOpen}
      className="flex min-w-0 flex-col gap-1"
    >
      <CollapsibleTrigger className="group/steps inline-flex w-fit max-w-full min-w-0 items-center gap-1.5 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg:not([class*='size-'])]:size-3.5">
        <StepIcon status={summaryStatus} />
        <span className="min-w-0 truncate">
          {counted(steps.length, "agent step")}
          {live ? (
            <>
              {" · "}
              <span className="font-medium text-foreground">
                {live.tool ?? "tool"}
              </span>
              {` ${TOOL_STATUS[live.status]}`}
            </>
          ) : null}
          {failed > 0 ? (
            <span className="text-destructive">{` · ${failed} failed`}</span>
          ) : null}
        </span>
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          aria-hidden
          className="size-3 shrink-0 transition-transform group-data-[panel-open]/steps:rotate-180 motion-reduce:transition-none"
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-1 border-l pl-3">
        {steps.map((step) => (
          <AssistantAgentTool
            key={step.key}
            tool={step.tool}
            status={step.status}
          />
        ))}
      </CollapsibleContent>
    </Collapsible>
  )
}

/**
 * An external agent asked to act on the machine, and Oxyn refused.
 *
 * There is deliberately no button to allow it: Oxyn cannot show which file or
 * which command is at stake, and a confirmation that does not say what it
 * allows is worse than a refusal (ADR-0026).
 */
export function AssistantPermissionRefused({
  action,
  reason,
}: {
  action: string
  reason: string
}) {
  return (
    <Marker
      data-slot="assistant-permission-refused"
      role="note"
      className="items-start text-xs"
    >
      <MarkerIcon className="mt-0.5">
        <HugeiconsIcon icon={SquareLock02Icon} strokeWidth={2} />
      </MarkerIcon>
      <MarkerContent className="flex flex-col gap-0.5">
        <span className="font-medium text-foreground">
          The agent asked to <span className="font-mono">{action}</span> on this
          machine. Oxyn refused.
        </span>
        <span>{reason}</span>
      </MarkerContent>
    </Marker>
  )
}

/**
 * Oxyn reading, from the server, the structure the assistant needs and its
 * catalog did not hold — then what it read and what is still missing
 * (ADR-0036). A step, like an agent's: not a live region.
 */
export function AssistantCatalogRead({ entry }: { entry: CatalogEntry }) {
  const partial =
    entry.failed > 0 ||
    entry.notLoaded > 0 ||
    entry.unlisted > 0 ||
    entry.stopped !== null
  return (
    <Marker
      data-slot="assistant-catalog-read"
      data-state={entry.reading ? "reading" : partial ? "partial" : "read"}
      className="items-start text-xs"
    >
      <MarkerIcon>
        {entry.reading ? (
          <Spinner />
        ) : (
          <HugeiconsIcon
            icon={partial ? InformationCircleIcon : CheckmarkCircle02Icon}
            strokeWidth={2}
          />
        )}
      </MarkerIcon>
      <MarkerContent>{catalogLine(entry)}</MarkerContent>
    </Marker>
  )
}

/** The earlier exchanges are not part of what answers from here. */
export function AssistantMemoryReset({ reason }: { reason: MemoryReset }) {
  return (
    <Marker
      data-slot="assistant-memory-reset"
      variant="border"
      role="note"
      className="items-start text-xs"
    >
      <MarkerIcon>
        <HugeiconsIcon
          icon={
            reason === "tierChanged" ? SquareLock02Icon : InformationCircleIcon
          }
          strokeWidth={2}
        />
      </MarkerIcon>
      <MarkerContent>{MEMORY_RESET_LINES[reason]}</MarkerContent>
    </Marker>
  )
}

/**
 * Discreet: a waiting step, not an alarm — and not a live region either. The
 * panel announces its state once, in one place; a marker per step read aloud
 * would drown the answer.
 */
export function AssistantWaiting({ label }: { label: string }) {
  return (
    <Marker className="text-xs">
      <MarkerIcon>
        <HugeiconsIcon icon={Clock01Icon} strokeWidth={2} />
      </MarkerIcon>
      <MarkerContent className="shimmer motion-reduce:shimmer-none">
        {label}
      </MarkerContent>
    </Marker>
  )
}
