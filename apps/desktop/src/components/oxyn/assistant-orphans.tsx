import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowDown01Icon } from "@hugeicons/core-free-icons"

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import type { OrphanThreadSummary } from "@/lib/ipc/ai"

/** Nothing is drawn until the list is read: most workspaces have none. */
export type OrphanThreadsState =
  | { status: "ready"; items: Array<OrphanThreadSummary> }
  | { status: "error"; message: string }

const UNTITLED = "Untitled conversation"
const UNNAMED = "a deleted connection"

function when(ms: number) {
  if (!Number.isFinite(ms) || ms <= 0) return ""
  return new Date(ms).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  })
}

/**
 * The conversations of connections deleted since, read-only.
 *
 * Deleting a connection keeps its conversations in the workspace, under the
 * connection's name (docs/UX-SPEC.md, « Une conversation reste dans le
 * workspace »). Without this list they would be kept where nobody can see
 * them. Rows are text, not buttons: nothing here opens, renames or deletes
 * one, and nothing here can send one to a model.
 *
 * Draws nothing while there are none: a folded « 0 » is noise in every
 * workspace that never deleted a connection.
 */
export function AssistantOrphans({ state }: { state: OrphanThreadsState }) {
  if (state.status === "ready" && state.items.length === 0) return null
  const count = state.status === "ready" ? state.items.length : null

  return (
    <Collapsible
      data-slot="assistant-orphans"
      className="flex min-w-0 flex-col gap-1"
    >
      <CollapsibleTrigger className="group/orphans inline-flex w-fit items-center gap-1.5 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
        <span>From deleted connections</span>
        {count !== null ? (
          <span className="tabular-nums">· {count}</span>
        ) : null}
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0 transition-transform group-data-[panel-open]/orphans:rotate-180 motion-reduce:transition-none"
          aria-hidden
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-1.5">
        <p className="text-xs text-muted-foreground">
          Kept in this workspace under the name their connection had. Read-only:
          their connection is gone, so they cannot be continued here.
        </p>
        {state.status === "error" ? (
          <p role="alert" className="text-xs text-destructive" data-selectable>
            They could not be listed: {state.message}
          </p>
        ) : null}
        {state.status === "ready" ? (
          <ul
            aria-label="Conversations of deleted connections"
            className="flex flex-col gap-0.5"
          >
            {state.items.map((item) => (
              <li
                key={item.id}
                className="flex min-w-0 flex-col gap-0.5 rounded-md px-2 py-1.5"
              >
                <span
                  className="min-w-0 truncate text-sm"
                  dir="auto"
                  title={item.title || UNTITLED}
                >
                  {item.title || UNTITLED}
                </span>
                <span className="min-w-0 truncate text-xs text-muted-foreground">
                  on <bdi>{item.connectionName ?? UNNAMED}</bdi> ·{" "}
                  <span className="tabular-nums">
                    {when(item.updatedAtMs)} · {item.exchanges} exchange(s)
                  </span>
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </CollapsibleContent>
    </Collapsible>
  )
}
