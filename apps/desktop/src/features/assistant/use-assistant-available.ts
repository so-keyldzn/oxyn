import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import { assistantEntry } from "./availability"
import type { AssistantEntry } from "./availability"
import { agentOptionsQuery, agentsQuery, providersQuery } from "./queries"
import type { OpenConnection } from "@/lib/ipc/types"
import { queryLanguageOf } from "@/lib/query-language"

/**
 * Whether the assistant exists for this connection.
 *
 * `absent` — no provider and no agent declared, or not read yet — means no AI
 * entry at all anywhere: no button, no badge, no greyed invitation
 * (docs/UX-SPEC.md, « The AI workspace only exists if it has been configured »).
 * The tier is read from the connection passed in: a screen that edits the
 * connection must pass the refreshed one (I-04).
 *
 * On a session that speaks another language than SQL, the agent roles offered
 * for the connection are read too: the assistant opens there only when an
 * agent writes that language (ADR-0065). A SQL session never waits for them.
 */
export function useAssistantAvailable(
  open: Pick<OpenConnection, "connection" | "privacyTier" | "capabilities">
): AssistantEntry {
  const providers = useQuery(providersQuery)
  const agents = useQuery(agentsQuery)
  const language = queryLanguageOf(open.capabilities)
  const roles = useQuery({
    ...agentOptionsQuery(open.connection),
    enabled: open.connection !== "" && language !== null && language !== "sql",
  })
  return React.useMemo(
    () =>
      assistantEntry({
        providers: providers.data,
        agents: agents.data,
        unreadable: providers.isError || agents.isError,
        tier: open.privacyTier,
        capabilities: open.capabilities,
        // A failed listing reads as no agent: the backend would refuse the
        // question for the same reason, and no retry is made (front.md).
        roles: roles.isError ? [] : roles.data,
      }),
    [
      providers.data,
      providers.isError,
      agents.data,
      agents.isError,
      open.privacyTier,
      open.capabilities,
      roles.data,
      roles.isError,
    ]
  )
}
