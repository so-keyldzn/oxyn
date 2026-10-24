// Whether the assistant exists for a connection, and who may answer there.
//
// A pure function of what the backend declared and of the connection's tier,
// so the rule of docs/UX-SPEC.md « The AI workspace only exists if it has been
// configured » is a unit test, not a hope. It is an affordance and never a
// gate: the backend re-checks the tier against a reach it measures itself at
// the moment of the question (I-04).

import type {
  AgentOption,
  DeclaredProvider,
  DestinationChoice,
  ExternalAgent,
  ProviderReach,
  ReasoningEffort,
} from "@/lib/ipc/ai"
import type { PrivacyTier } from "@/lib/ipc/types"
import { QUERY_LANGUAGE_LABELS, queryLanguageOf } from "@/lib/query-language"
import type { QueryLanguageName } from "@/lib/query-language"

export interface DestinationOption {
  /** `provider:<id>` or `agent:<id>`: unique across both kinds. */
  key: string
  kind: "provider" | "agent"
  id: string
  label: string
  model: string | null
  reach: ProviderReach
  usable: boolean
  /** Why this tier refuses it. */
  reason: string | null
  /**
   * An agent Oxyn cannot confine — declared by hand, not from a preset — and
   * so cannot keep from acting on the machine by itself (ADR-0032).
   */
  unconfined?: boolean
}

export type AssistantEntry =
  /** Nothing is drawn: no badge, no greyed control, no invitation. */
  | { status: "absent" }
  /** Drawn, inert, and saying why. */
  | {
      status: "disabled"
      reason: string
      destinations: Array<DestinationOption>
    }
  | { status: "enabled"; destinations: Array<DestinationOption> }

/** The backend's refusal for the same session (`agents::NO_LANGUAGE`). */
export const NO_LANGUAGE =
  "This session declares no single query language, and the assistant writes in the language the session speaks: Oxyn cannot tell which one to use."

/** Why the assistant is closed on a session that speaks `language`. */
export function noAgentFor(language: QueryLanguageName): string {
  return `No agent writes ${QUERY_LANGUAGE_LABELS[language]} yet, and the assistant only writes what an agent knows.`
}

/** While the agents of a language other than SQL are being listed. */
export function readingAgentsFor(language: QueryLanguageName): string {
  return `Looking for an agent that writes ${QUERY_LANGUAGE_LABELS[language]}…`
}

/**
 * Whether an agent writes the session's language (ADR-0065): data, not a
 * list in the code — an agent file shipped or written by the user opens the
 * assistant on its language.
 *
 * SQL needs no list: the shipped SQL agent is offered on every SQL session,
 * and the entry does not wait for the picker's list to draw. Elsewhere, an
 * agent offered for the connection that is not in error (`ai_list_agents`
 * lists only the agents of the connection's language, and broken files).
 */
function agentFor(
  language: QueryLanguageName,
  offered: ReadonlyArray<AgentOption> | undefined
): "yes" | "no" | "unknown" {
  if (language === "sql") return "yes"
  if (offered === undefined) return "unknown"
  return offered.some((agent) => agent.error === null) ? "yes" : "no"
}

export const UNREADABLE =
  "The declared providers could not be read from local state."

/** Names both kinds: under `Local`, both are closed for the same reason. */
export const NO_USABLE_DESTINATION =
  "This connection's privacy tier only admits a provider that resolved to this machine, and no declared provider is local. An external agent cannot serve it either: nothing says where it sends data."

const REMOTE_REFUSED =
  "This connection is local-only, and this endpoint leaves the machine or could not be resolved."

const AGENT_REFUSED =
  "This connection is local-only, and Oxyn cannot see where an external agent sends data."

export function destinationOptions(
  providers: ReadonlyArray<DeclaredProvider>,
  agents: ReadonlyArray<ExternalAgent>,
  tier: PrivacyTier
): Array<DestinationOption> {
  const remoteAllowed = tier !== "local"
  return [
    ...providers.map((provider) => {
      // `unresolved` gets no benefit of the doubt.
      const usable = provider.reach === "local" || remoteAllowed
      return {
        key: `provider:${provider.id}`,
        kind: "provider" as const,
        id: provider.id,
        label: provider.label,
        model: provider.model,
        reach: provider.reach,
        usable,
        reason: usable ? null : REMOTE_REFUSED,
      }
    }),
    ...agents.map((agent) => ({
      key: `agent:${agent.id}`,
      kind: "agent" as const,
      id: agent.id,
      label: agent.label,
      model: null,
      reach: "unresolved" as const,
      usable: remoteAllowed,
      reason: remoteAllowed ? null : AGENT_REFUSED,
      unconfined: !agent.confined,
    })),
  ]
}

export function assistantEntry({
  providers,
  agents,
  unreadable,
  tier,
  capabilities,
  roles,
}: {
  /** `undefined` until read: drawn like « none », rather than flickering in. */
  providers: ReadonlyArray<DeclaredProvider> | undefined
  agents: ReadonlyArray<ExternalAgent> | undefined
  /** Not knowing is not knowing there is nothing: the entry stays, inert. */
  unreadable: boolean
  tier: PrivacyTier
  capabilities: ReadonlyArray<string>
  /**
   * The agent roles offered for this connection (`ai_list_agents`);
   * `undefined` until read, or when the session speaks SQL.
   */
  roles?: ReadonlyArray<AgentOption>
}): AssistantEntry {
  if (unreadable) {
    return { status: "disabled", reason: UNREADABLE, destinations: [] }
  }
  const destinations = destinationOptions(providers ?? [], agents ?? [], tier)
  if (destinations.length === 0) return { status: "absent" }
  const language = queryLanguageOf(capabilities)
  if (language === null) {
    return { status: "disabled", reason: NO_LANGUAGE, destinations }
  }
  const written = agentFor(language, roles)
  if (written !== "yes") {
    const reason =
      written === "unknown" ? readingAgentsFor(language) : noAgentFor(language)
    return { status: "disabled", reason, destinations }
  }
  if (!destinations.some((destination) => destination.usable)) {
    return { status: "disabled", reason: NO_USABLE_DESTINATION, destinations }
  }
  return { status: "enabled", destinations }
}

/**
 * The destination a question goes to by default.
 *
 * A provider before an agent: it is the only one whose destination Oxyn can
 * verify (ADR-0026).
 */
export function defaultDestination(
  destinations: ReadonlyArray<DestinationOption>
): DestinationOption | null {
  return (
    destinations.find(
      (option) => option.usable && option.kind === "provider"
    ) ??
    destinations.find((option) => option.usable) ??
    null
  )
}

/** The destination the user picked, while it is still offered; else the default. */
export function selectedDestination(
  destinations: ReadonlyArray<DestinationOption>,
  chosenKey: string | null
): DestinationOption | null {
  return (
    destinations.find((option) => option.key === chosenKey) ??
    defaultDestination(destinations)
  )
}

/**
 * Where the destination a question would go to resolved, for the tier label;
 * `null` when nothing can be sent. Read from the declared providers, never
 * assumed: without one, the label says no « Cloud » (docs/UX-SPEC.md).
 */
export function answeringReach(
  entry: AssistantEntry,
  chosenKey: string | null
): ProviderReach | null {
  if (entry.status !== "enabled") return null
  const selected = selectedDestination(entry.destinations, chosenKey)
  return selected?.usable ? selected.reach : null
}

export function destinationChoice(
  option: DestinationOption,
  model: string | null,
  effort: ReasoningEffort | null = null
): DestinationChoice {
  return option.kind === "provider"
    ? { kind: "provider", id: option.id, model, effort }
    : { kind: "agent", id: option.id }
}
