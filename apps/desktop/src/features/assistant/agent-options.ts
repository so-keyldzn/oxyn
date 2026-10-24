import type { DestinationOption } from "./availability"
import type { AgentOption } from "@/lib/ipc/ai"

/** The shipped identity is stable, including in pre-picker conversations. */
export const SQL_AGENT_ID = "0199a3c0-0000-7000-8000-000000000001"

export const SQL_ONLY: ReadonlyArray<AgentOption> = [
  {
    id: SQL_AGENT_ID,
    name: "SQL",
    description: "Writes, fixes and explains queries on the open connection.",
    origin: "shipped",
    error: null,
    disabledDestinations: [],
  },
]

/**
 * The agent a blank thread runs, as the backend chooses it
 * (`AgentCatalog::default_for`): the SQL agent when it is offered — every SQL
 * session (ADR-0055) —, otherwise the first agent offered for the session's
 * language that is not in error, a shipped one before a user's (ADR-0065).
 * The SQL agent while nothing is listed, as before the list arrives.
 *
 * The backend chooses for the recipient the question goes to, whereas the list
 * is the union over every declared destination. `destinations` are those the
 * tier leaves usable, the one the question would go to first: the agent is the
 * first that is written for one of them, the first destination deciding. An
 * agent offered only where the tier refuses would leave nothing to answer
 * with, another agent being available. Without a usable destination, the
 * first valid agent, as before.
 */
export function defaultAgentId(
  agents: ReadonlyArray<AgentOption>,
  destinations: ReadonlyArray<DestinationOption> = []
): string {
  if (agents.length === 0 || agents.some((agent) => agent.id === SQL_AGENT_ID))
    return SQL_AGENT_ID
  const valid = agents.filter((agent) => agent.error === null)
  const ranked = [
    ...valid.filter((agent) => agent.origin === "shipped"),
    ...valid.filter((agent) => agent.origin !== "shipped"),
  ]
  const usable = destinations.filter((option) => option.usable)
  for (const option of usable) {
    const fit = ranked.find((agent) => !refuses(agent, option))
    if (fit) return fit.id
  }
  return ranked.at(0)?.id ?? SQL_AGENT_ID
}

function refuses(agent: AgentOption, option: DestinationOption): boolean {
  return agent.disabledDestinations.some(
    (ref) => ref.kind === option.kind && ref.id === option.id
  )
}

/**
 * The destinations as "Who answers" offers them to a conversation running
 * `agent`: those it is not written for stay listed, disabled, with their
 * reason (ADR-0049 § 6, UX-SPEC). A destination the tier already refuses
 * keeps the tier's reason. Changing destination never switches the agent:
 * the reason says how to use another one.
 */
export function restrictedToAgent(
  destinations: ReadonlyArray<DestinationOption>,
  agent: AgentOption | undefined
): Array<DestinationOption> {
  return destinations.map((option) =>
    option.usable && agent && refuses(agent, option)
      ? {
          ...option,
          usable: false,
          reason: `The ${agent.name} agent is not written for it. Start a new conversation with another agent to use it.`,
        }
      : option
  )
}
