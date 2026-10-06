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
  const refused = agent?.disabledDestinations ?? []
  return destinations.map((option) =>
    option.usable &&
    refused.some((ref) => ref.kind === option.kind && ref.id === option.id)
      ? {
          ...option,
          usable: false,
          reason: `The ${agent?.name ?? "current"} agent is not written for it. Start a new conversation with another agent to use it.`,
        }
      : option
  )
}
