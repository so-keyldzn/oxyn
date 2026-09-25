import type { PrunedHistory } from "@/lib/ipc/ai"

function mebibytes(bytes: number) {
  return `${Math.round(bytes / (1024 * 1024))} MiB`
}

/** The rule as the backend applied it, from its own numbers. Pure, so it is tested. */
export function prunedSentence(pruned: PrunedHistory) {
  const count =
    pruned.conversations === 1
      ? "1 conversation"
      : `${pruned.conversations} conversations`
  const age =
    pruned.maxAgeDays === null
      ? ""
      : `, and removes those idle for ${pruned.maxAgeDays} days`
  return `Oxyn removed ${count} when it started. It keeps the ${pruned.maxConversations} most recent, within ${mebibytes(pruned.maxBytes)}${age}.`
}

/**
 * What the launch removed from this workspace's history, said once in the
 * list rather than only in a log nobody reads (docs/UX-SPEC.md). A thread
 * that vanished without a word reads as a defect; one removed by a stated
 * rule reads as the rule.
 */
export function AssistantPrunedNote({ pruned }: { pruned: PrunedHistory }) {
  return (
    <p
      data-slot="assistant-history-pruned"
      className="rounded-md border bg-muted/40 px-2.5 py-1.5 text-xs text-muted-foreground"
    >
      {prunedSentence(pruned)}
    </p>
  )
}
