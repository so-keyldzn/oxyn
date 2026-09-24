import { describe, expect, it } from "vitest"

import { answeringReach, destinationOptions } from "./availability"
import type { AssistantEntry } from "./availability"
import type { DeclaredProvider, ExternalAgent } from "@/lib/ipc/ai"

const provider = (
  id: string,
  reach: DeclaredProvider["reach"]
): DeclaredProvider => ({
  id,
  label: id,
  kind: "openai_compatible",
  endpoint: "http://example.invalid",
  endpointRedacted: false,
  model: "model",
  keyConfigured: false,
  reach,
  measuredAtMs: 0,
})

const agent: ExternalAgent = {
  id: "agent-1",
  label: "Agent",
  command: "agent",
  argCount: 0,
  envNames: [],
  preset: null,
  confined: true,
}

const enabled = (
  providers: Array<DeclaredProvider>,
  agents: Array<ExternalAgent> = []
): AssistantEntry => ({
  status: "enabled",
  destinations: destinationOptions(providers, agents, "metadata"),
})

describe("where the tier label says the answer comes from", () => {
  it("is the default provider's measured reach", () => {
    expect(answeringReach(enabled([provider("cloud", "remote")]), null)).toBe(
      "remote"
    )
    expect(answeringReach(enabled([provider("ollama", "local")]), null)).toBe(
      "local"
    )
  })

  it("follows the destination picked in the panel", () => {
    const entry = enabled([
      provider("cloud", "remote"),
      provider("ollama", "local"),
    ])
    expect(answeringReach(entry, "provider:ollama")).toBe("local")
  })

  it("claims no reach for an agent, nothing sendable or nothing declared", () => {
    // An agent's destination is not measured: `unresolved`, never « Cloud ».
    expect(answeringReach(enabled([], [agent]), null)).toBe("unresolved")
    expect(answeringReach({ status: "absent" }, null)).toBeNull()
    expect(
      answeringReach(
        { status: "disabled", reason: "no", destinations: [] },
        null
      )
    ).toBeNull()
  })
})
