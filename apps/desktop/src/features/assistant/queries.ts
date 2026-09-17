import { queryOptions } from "@tanstack/react-query"

import { ai } from "@/lib/ipc/ai"

export const aiKeys = {
  all: ["ai"] as const,
  providers: ["ai", "providers"] as const,
  agents: ["ai", "agents"] as const,
  models: (provider: string) => ["ai", "models", provider] as const,
  presets: ["ai", "presets"] as const,
}

/**
 * The declared providers, classified by the backend.
 *
 * Kept briefly: a classification is a DNS resolution of this session, and the
 * settings screen invalidates the list after every write, which is what makes
 * the first provider appear without a restart (docs/UX-SPEC.md).
 */
export const providersQuery = queryOptions({
  queryKey: aiKeys.providers,
  queryFn: () => ai.providers(),
  staleTime: 30_000,
})

export const agentsQuery = queryOptions({
  queryKey: aiKeys.agents,
  queryFn: () => ai.externalAgents(),
  staleTime: 30_000,
})

/**
 * The ready-made agent declarations. Static values from the backend, which is
 * where their versions are recorded (I-12): nothing here looks at the machine.
 */
export const presetsQuery = queryOptions({
  queryKey: aiKeys.presets,
  queryFn: () => ai.agentPresets(),
  staleTime: Number.POSITIVE_INFINITY,
})

/** One request to the endpoint, made only when someone asks for the list. */
export function modelsQuery(provider: string) {
  return queryOptions({
    queryKey: aiKeys.models(provider),
    queryFn: () => ai.providerModels(provider),
    staleTime: Number.POSITIVE_INFINITY,
  })
}
