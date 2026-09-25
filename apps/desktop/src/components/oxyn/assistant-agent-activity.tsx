import { HugeiconsIcon } from "@hugeicons/react"
import {
  CancelCircleIcon,
  CheckmarkCircle02Icon,
  Clock01Icon,
  InformationCircleIcon,
  SquareLock02Icon,
} from "@hugeicons/core-free-icons"

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
  const kind = tool ?? "tool"
  return (
    <Marker
      data-slot="assistant-agent-tool"
      data-status={status}
      className="text-xs"
    >
      <MarkerIcon>
        {status === "running" || status === "pending" ? (
          <Spinner />
        ) : (
          <HugeiconsIcon
            icon={
              status === "completed" ? CheckmarkCircle02Icon : CancelCircleIcon
            }
            strokeWidth={2}
            className={cn(status === "failed" && "text-destructive")}
          />
        )}
      </MarkerIcon>
      <MarkerContent className="font-mono">{`Agent step · ${kind} · ${TOOL_STATUS[status]}`}</MarkerContent>
    </Marker>
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
