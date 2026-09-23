import type { ContextWindow, Usage } from "@/features/assistant/transcript"
import type { ModelCost } from "@/lib/ipc/ai"

const compact = new Intl.NumberFormat(undefined, {
  notation: "compact",
  maximumFractionDigits: 1,
})

function money(amount: number, currency: string) {
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
      maximumSignificantDigits: 3,
    }).format(amount)
  } catch {
    // A currency code the runtime does not know is shown as given.
    return `${amount.toPrecision(3)} ${currency}`
  }
}

/**
 * The cost of a run, from what the provider published — never from a price
 * written in Oxyn (I-12). `null` when the provider publishes none.
 *
 * Cache reads and writes are not priced here: providers bill them at rates
 * the model listing does not carry.
 */
export function usageCost(usage: Usage, cost: ModelCost | null) {
  if (!cost) return null
  return (
    (usage.input * cost.inputPerMillion +
      usage.output * cost.outputPerMillion) /
    1_000_000
  )
}

/**
 * What a run consumed, said discreetly under the answer.
 *
 * An undeclared figure is left out, not shown as zero: a cache nobody reported
 * is not a cache that was not used.
 */
export function AssistantUsage({
  usage,
  contextWindow,
  cost,
}: {
  usage: Usage | null
  contextWindow: ContextWindow | null
  cost: ModelCost | null
}) {
  const parts: Array<{ key: string; text: string; title: string }> = []
  if (usage) {
    parts.push({
      key: "in",
      text: `${compact.format(usage.input)} in`,
      title: `${usage.input.toLocaleString()} input tokens`,
    })
    parts.push({
      key: "out",
      text: `${compact.format(usage.output)} out`,
      title: `${usage.output.toLocaleString()} output tokens`,
    })
    if (usage.cacheRead !== null) {
      parts.push({
        key: "read",
        text: `${compact.format(usage.cacheRead)} cache read`,
        title: `${usage.cacheRead.toLocaleString()} tokens read from the provider's cache`,
      })
    }
    if (usage.cacheWrite !== null) {
      parts.push({
        key: "write",
        text: `${compact.format(usage.cacheWrite)} cache write`,
        title: `${usage.cacheWrite.toLocaleString()} tokens written to the provider's cache`,
      })
    }
    const amount = usageCost(usage, cost)
    if (amount !== null && cost) {
      parts.push({
        key: "cost",
        text: `≈ ${money(amount, cost.currency)}`,
        title: "Estimated from the provider's published price, cache excluded",
      })
    }
  }
  if (contextWindow) {
    parts.push({
      key: "context",
      text: `context ${compact.format(contextWindow.used)} / ${compact.format(contextWindow.size)}`,
      title: `${contextWindow.used.toLocaleString()} of ${contextWindow.size.toLocaleString()} tokens in the agent's context`,
    })
    if (contextWindow.cost) {
      parts.push({
        key: "agent-cost",
        text: money(contextWindow.cost.amount, contextWindow.cost.currency),
        title: "Session cost, as the agent reports it",
      })
    }
  }
  if (parts.length === 0) return null

  return (
    <p
      data-slot="assistant-usage"
      className="flex flex-wrap items-center gap-x-1.5 text-[11px] text-muted-foreground tabular-nums"
    >
      <span className="sr-only">Usage: </span>
      {parts.map((part, index) => (
        <span key={part.key} title={part.title} className="cursor-help">
          {index > 0 ? (
            <span aria-hidden className="mr-1.5">
              ·
            </span>
          ) : null}
          {part.text}
        </span>
      ))}
    </p>
  )
}
