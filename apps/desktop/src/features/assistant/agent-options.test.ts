import { describe, expect, it } from "vitest"

import { restrictedToAgent } from "./agent-options"
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
