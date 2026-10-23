import { describe, expect, it } from "vitest"

import {
  SQL_AGENT_ID,
  SQL_ONLY,
  defaultAgentId,
  restrictedToAgent,
} from "./agent-options"
import type { DestinationOption } from "./availability"
import type { AgentOption } from "@/lib/ipc/ai"

function destination(
  kind: "provider" | "agent",
  id: string,
  usable = true,
  reason: string | null = null
): DestinationOption {
  return {
    key: `${kind}:${id}`,
    kind,
    id,
    label: id,
    model: null,
    reach: "remote",
    usable,
    reason,
  }
}

const localOnly: AgentOption = {
  id: "0199a3c0-0000-7000-8000-0000000000a2",
  name: "Small model",
  description: "",
  origin: "user",
  error: null,
  disabledDestinations: [
    { kind: "provider", id: "hosted" },
    { kind: "agent", id: "codex" },
  ],
}

describe("restrictedToAgent", () => {
  it("disables, with a reason, the destinations the agent is not written for", () => {
    const [local, hosted, codex] = restrictedToAgent(
      [
        destination("provider", "local"),
        destination("provider", "hosted"),
        destination("agent", "codex"),
      ],
      localOnly
    )
    expect(local).toMatchObject({ usable: true, reason: null })
    expect(hosted?.usable).toBe(false)
    expect(hosted?.reason).toMatch(/Small model agent is not written for it/)
    expect(codex?.usable).toBe(false)
  })

  it("matches on kind and id together", () => {
    const [agentNamedHosted] = restrictedToAgent(
      [destination("agent", "hosted")],
      localOnly
    )
    expect(agentNamedHosted?.usable).toBe(true)
  })

  it("keeps the tier's reason when the tier already refuses", () => {
    const tierReason = "This connection is local-only."
    const [hosted] = restrictedToAgent(
      [destination("provider", "hosted", false, tierReason)],
      localOnly
    )
    expect(hosted?.reason).toBe(tierReason)
  })

  it("changes nothing without a known agent", () => {
    const options = [destination("provider", "hosted")]
    expect(restrictedToAgent(options, undefined)).toEqual(options)
  })
})

describe("defaultAgentId", () => {
  const keys = (
    name: string,
    origin: AgentOption["origin"],
    error: string | null = null
  ): AgentOption => ({
    id: `id-${name}`,
    name,
    description: "",
    origin,
    error,
    disabledDestinations: [],
  })

  it("is the SQL agent wherever it is offered, whatever is listed first", () => {
    expect(defaultAgentId([localOnly, ...SQL_ONLY])).toBe(SQL_AGENT_ID)
    expect(defaultAgentId([])).toBe(SQL_AGENT_ID)
  })

  it("is the shipped agent of the language before any user file", () => {
    expect(
      defaultAgentId([
        keys("broken.md", "user", "broken.md, line 1: …"),
        keys("Alpha", "user"),
        keys("Redis", "shipped"),
      ])
    ).toBe("id-Redis")
    expect(defaultAgentId([keys("Alpha", "user"), keys("Zeta", "user")])).toBe(
      "id-Alpha"
    )
  })
})
