// Which reasoning effort a question to a built-in provider carries.
//
// Only what the chosen model declares. The backend checks again and refuses
// the rest; this keeps a choice made for one model from being sent with
// another, where it would come back as a refusal instead of an answer.

import type { ModelChoice, ReasoningEffort } from "@/lib/ipc/ai"

/** The words shown for each level, in the provider's order. */
export const EFFORT_LABELS: Record<ReasoningEffort, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
}

/** The efforts `model` declares; empty when it declares none or is not listed. */
export function declaredEfforts(
  models: ReadonlyArray<ModelChoice> | null,
  model: string | null
): ReadonlyArray<ReasoningEffort> {
  return models?.find((choice) => choice.id === model)?.reasoningEfforts ?? []
}

/**
 * The effort to send: the one chosen, if the current model declares it, and
 * none otherwise — including before the model list has been read.
 */
export function effortToSend(
  chosen: ReasoningEffort | null,
  models: ReadonlyArray<ModelChoice> | null,
  model: string | null
): ReasoningEffort | null {
  return chosen !== null && declaredEfforts(models, model).includes(chosen)
    ? chosen
    : null
}
