import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  CheckmarkCircle02Icon,
  CircleIcon,
  TaskDaily01Icon,
} from "@hugeicons/core-free-icons"

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { Spinner } from "@/components/ui/spinner"
import type {
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
} from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

/**
 * The plan of a run, in the shape the Agent Client Protocol gives it.
 *
 * The three types come from the boundary schema (`lib/ipc/ai.ts`), not from a
 * second declaration here: a boundary type written twice is the mirror that
 * front.md exists to remove (ADR-0031).
 *
 * The lists are closed in v1 — three priorities, three statuses. There is no
 * « failed » and no « abandoned »: an agent that gives up rewrites its plan.
 *
 * `content` is model output, therefore hostile input: rendered as React text,
 * never as markup (docs/SECURITY.md).
 */
export type {
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
} from "@/lib/ipc/ai"

const STATUS: Record<PlanEntryStatus, string> = {
  pending: "To do",
  inProgress: "In progress",
  completed: "Done",
}

const PRIORITY: Record<PlanEntryPriority, string> = {
  high: "high priority",
  medium: "medium priority",
  low: "low priority",
}

/**
 * Reads a word of the protocol without trusting it.
 *
 * The type says the word is one of three; the protocol only says so for v1.
 * A word gained in a later version would index these tables to `undefined`,
 * and the next property access would blank the panel — a whole answer lost to
 * one unknown string. `Object.hasOwn` is the runtime check the type cannot
 * make, and `Record<PlanEntry…, string>` is kept rather than widened to
 * `Record<string, string>` so that adding a word to the union still fails the
 * build here.
 */
function wordOf<TWord extends string>(
  table: Record<TWord, string>,
  word: TWord,
  fallback: string
) {
  return Object.hasOwn(table, word) ? table[word] : fallback
}

function StepIcon({ status }: { status: PlanEntryStatus }) {
  if (status === "inProgress") return <Spinner className="size-3.5" />
  return (
    <HugeiconsIcon
      icon={status === "completed" ? CheckmarkCircle02Icon : CircleIcon}
      strokeWidth={2}
      className={cn(
        "size-3.5",
        status === "completed" ? "text-success" : "text-muted-foreground"
      )}
      aria-hidden
    />
  )
}

function Step({ entry, position }: { entry: PlanEntry; position: number }) {
  const status = wordOf(STATUS, entry.status, "To do")
  const priority = wordOf(PRIORITY, entry.priority, "priority not stated")
  return (
    <li
      data-slot="assistant-plan-step"
      data-status={entry.status}
      data-priority={entry.priority}
      // `aria-current` is what tells a screen reader where the run stands;
      // the spinner is decorative and says nothing on its own.
      aria-current={entry.status === "inProgress" ? "step" : undefined}
      className="flex min-w-0 items-start gap-2"
    >
      <span className="mt-0.5 flex shrink-0 items-center">
        <StepIcon status={entry.status} />
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span
          className={cn(
            "min-w-0 wrap-break-word",
            entry.status === "pending" && "text-muted-foreground"
          )}
        >
          {/* `dir="auto"` so a right-to-left step does not drag the
              punctuation of its neighbours across the list. */}
          <span dir="auto">{entry.content}</span>
          {/* Status and priority in words, for a reader who gets neither the
              icon nor the marker. Every entry carries both: only `high` is
              worth a visible marker, and none of it should be lost for that. */}
          <span className="sr-only">{` — step ${position}, ${status}, ${priority}`}</span>
        </span>
      </span>
      {entry.priority === "high" ? (
        <span className="shrink-0 text-[0.6875rem] text-warning" aria-hidden>
          High
        </span>
      ) : null}
    </li>
  )
}

/**
 * The steps the agent says it is going through, in its order.
 *
 * Three properties of the protocol decide everything here, and none of them
 * is a detail.
 *
 * **The agent replaces the whole plan on every send; it never sends a delta.**
 * So this component accumulates nothing and reconciles nothing: it renders the
 * list it was given. Replacement is the ordinary case, not an edge one.
 *
 * **An entry has no identifier in v1: its identity is its position.** The
 * React key is therefore the index — correct here, and only here, because
 * position *is* identity. It follows that the same position may carry
 * unrelated content and a different status between two sends, which is why
 * nothing animates between two plans: a transition would suggest a step
 * became another one.
 *
 * **A plan describes intent, not authority.** A step that names a command has
 * not been submitted by the fact of being listed, and nothing here runs
 * anything (I-07). What did go through the bus is drawn by `AssistantToolCall`,
 * with its connection and its approval.
 */
export function AssistantPlan({
  entries,
  running = false,
  defaultOpen = true,
}: {
  /** The whole plan as last received. Never merged with a previous one. */
  entries: ReadonlyArray<PlanEntry>
  /** The turn is still running — Oxyn's own fact, not a protocol state. */
  running?: boolean
  defaultOpen?: boolean
}) {
  const [open, setOpen] = React.useState(defaultOpen)

  // A plan with no step is not information: an empty frame would read as
  // something that failed to load. While a turn runs without one, the wait is
  // what there is to show.
  if (entries.length === 0) {
    if (!running) return null
    return (
      <p
        data-slot="assistant-plan"
        role="status"
        className="inline-flex w-fit items-center gap-1.5 py-0.5 text-xs text-muted-foreground"
      >
        <HugeiconsIcon
          icon={TaskDaily01Icon}
          strokeWidth={2}
          className="size-3.5"
          aria-hidden
        />
        <span className="shimmer motion-reduce:shimmer-none">
          Planning the steps…
        </span>
      </p>
    )
  }

  const done = entries.filter((entry) => entry.status === "completed").length

  return (
    <Collapsible
      data-slot="assistant-plan"
      open={open}
      onOpenChange={setOpen}
      className="flex min-w-0 flex-col text-sm"
    >
      <CollapsibleTrigger className="group/plan inline-flex w-fit items-center gap-1.5 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
        <HugeiconsIcon
          icon={TaskDaily01Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0"
          aria-hidden
        />
        <span>Plan</span>
        {/* Only the counter is live: announcing forty steps on every update
            would bury the one fact that changed. */}
        <span className="tabular-nums" aria-live="polite">
          · {done} of {entries.length} done
        </span>
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0 transition-transform group-data-[panel-open]/plan:rotate-180 motion-reduce:transition-none"
          aria-hidden
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="overflow-hidden">
        {/* Vertical only: a long step wraps, it never pushes a scrollbar
            across a 320 px panel. `tabIndex` because a region that scrolls
            and holds no control is unreachable from the keyboard otherwise. */}
        <ol
          aria-label="Plan steps"
          tabIndex={0}
          className="mt-1.5 flex max-h-64 min-w-0 flex-col gap-1.5 overflow-x-hidden overflow-y-auto border-l-2 pl-3 text-xs leading-5 outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {entries.map((entry, index) => (
            // The index is the key on purpose: a v1 entry has no identifier,
            // and its position is what identifies it.
            <Step key={index} entry={entry} position={index + 1} />
          ))}
        </ol>
      </CollapsibleContent>
    </Collapsible>
  )
}
