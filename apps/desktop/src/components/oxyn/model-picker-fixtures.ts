// Model lists for the stories: synthetic names, so that no story reads as a
// list of real models to pick from.

import type { ModelChoice, ModelListing } from "@/lib/ipc/ai"

function choice(id: string, displayName: string): ModelChoice {
  return {
    id,
    displayName,
    contextWindow: 200_000,
    cost: null,
    reasoningEfforts: [],
  }
}

export const draftModels: ReadonlyArray<ModelChoice> = [
  choice("model-orion-2", "Orion 2"),
  choice("model-lyra-1", "Lyra"),
  choice("model-vega-mini", "Vega mini"),
]

export const otherModels: ReadonlyArray<ModelChoice> = [
  choice("model-draco-5", "Draco 5"),
]

export function listed(models: ReadonlyArray<ModelChoice>): ModelListing {
  return {
    status: "ok",
    models: [...models],
    cached: false,
    fetchedAtMs: Date.UTC(2026, 9, 4, 9, 30),
  }
}
