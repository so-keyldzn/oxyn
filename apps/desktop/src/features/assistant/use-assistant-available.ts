import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import { assistantEntry } from "./availability"
import type { AssistantEntry } from "./availability"
import { agentsQuery, providersQuery } from "./queries"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * Whether the assistant exists for this connection.
 *
 * `absent` — no provider and no agent declared, or not read yet — means no AI
 * entry at all anywhere: no button, no badge, no greyed invitation
 * (docs/UX-SPEC.md, « Le workspace IA n'existe que s'il a été configuré »).
 * The tier is read from the connection passed in: a screen that edits the
 * connection must pass the refreshed one (I-04).
 */
export function useAssistantAvailable(
  open: Pick<OpenConnection, "privacyTier" | "capabilities">
): AssistantEntry {
  const providers = useQuery(providersQuery)
  const agents = useQuery(agentsQuery)
  return React.useMemo(
    () =>
      assistantEntry({
        providers: providers.data,
        agents: agents.data,
        unreadable: providers.isError || agents.isError,
        tier: open.privacyTier,
        capabilities: open.capabilities,
      }),
    [
      providers.data,
      providers.isError,
      agents.data,
      agents.isError,
      open.privacyTier,
      open.capabilities,
    ]
  )
}
