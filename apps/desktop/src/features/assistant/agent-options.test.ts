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

  describe("on a language that is not SQL, with the tier's usable destinations", () => {
    const remote = destination("provider", "remote")
    const local = destination("provider", "local")
    // Alpha is written for the remote provider only, Beta for the local one:
    // the union of what is offered lists both, one destination each.
    const alpha: AgentOption = {
      ...keys("Alpha", "user"),
      disabledDestinations: [{ kind: "provider", id: "local" }],
    }
    const beta: AgentOption = {
      ...keys("Beta", "user"),
      disabledDestinations: [{ kind: "provider", id: "remote" }],
    }

    it("skips an agent the only usable destination is refused to", () => {
      // A Local tier: the remote provider is already refused.
      const tierRefused = { ...remote, usable: false, reason: "Local-only." }
      expect(defaultAgentId([alpha, beta], [tierRefused, local])).toBe(
        "id-Beta"
      )
      expect(restrictedToAgent([tierRefused, local], alpha)).toSatisfy(
        (options: Array<DestinationOption>) =>
          options.every((option) => !option.usable)
      )
    })

    it("agrees with the backend for the destination the user picked", () => {
      expect(defaultAgentId([alpha, beta], [remote, local])).toBe("id-Alpha")
      expect(defaultAgentId([alpha, beta], [local, remote])).toBe("id-Beta")
    })

    it("falls back to another usable destination when none fits the first", () => {
      expect(defaultAgentId([alpha], [local, remote])).toBe("id-Alpha")
    })

    it("keeps the shipped agent before a user's among those that fit", () => {
      const shipped = keys("Redis", "shipped")
      expect(defaultAgentId([shipped, alpha, beta], [local])).toBe("id-Redis")
    })

    it("keeps the first valid agent when no destination is usable", () => {
      expect(defaultAgentId([alpha, beta], [])).toBe("id-Alpha")
    })

    it("leaves SQL alone", () => {
      expect(defaultAgentId([alpha, ...SQL_ONLY], [local])).toBe(SQL_AGENT_ID)
    })
  })
})
