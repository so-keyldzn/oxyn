import { queryOptions } from "@tanstack/react-query"

import { ai } from "@/lib/ipc/ai"
import type { AgentPresetId } from "@/lib/ipc/ai"

export const aiKeys = {
  all: ["ai"] as const,
  providers: ["ai", "providers"] as const,
  agents: ["ai", "agents"] as const,
  models: (provider: string) => ["ai", "models", provider] as const,
  presets: ["ai", "presets"] as const,
  detected: (preset: string) => ["ai", "detected", preset] as const,
  orphans: ["ai", "orphans"] as const,
  pruned: ["ai", "pruned"] as const,
}

/**
 * What the launch's prune removed. Settled once it is known: a launch prunes
 * once, and a `null` read before the prune ended is read again.
 */
export const prunedHistoryQuery = queryOptions({
  queryKey: aiKeys.pruned,
  queryFn: () => ai.prunedHistory(),
  staleTime: 0,
})

/**
 * The conversations of connections deleted since, for the history panel.
 *
 * Read again each time the list is shown: deleting a connection happens in
 * settings, and nothing tells the panel.
 */
export const orphanThreadsQuery = queryOptions({
  queryKey: aiKeys.orphans,
  queryFn: () => ai.orphanThreads(),
  staleTime: 0,
})

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

/**
 * What the machine has for one preset: a read of the usual directories, off the
 * UI thread in the backend, which never runs the program nor saves anything.
 *
 * Refetched each time the settings screen opens, since installing an agent in
 * a terminal while Oxyn runs is the ordinary way to get one.
 */
export function detectedAgentQuery(preset: AgentPresetId) {
  return queryOptions({
    queryKey: aiKeys.detected(preset),
    queryFn: () => ai.detectAgent(preset),
    staleTime: 0,
    // Opening the screen is the moment, not coming back to the window.
    refetchOnWindowFocus: false,
    retry: false,
  })
}

/** One request to the endpoint, made only when someone asks for the list. */
export function modelsQuery(provider: string) {
  return queryOptions({
    queryKey: aiKeys.models(provider),
    queryFn: () => ai.providerModels(provider),
    staleTime: Number.POSITIVE_INFINITY,
  })
}
