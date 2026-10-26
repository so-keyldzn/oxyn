import { describe, expect, it } from "vitest"

import { externalAgent, remoteProvider } from "./assistant-fixtures"
import { providerRows } from "./provider-list-model"
import type { AgentPresetDraft } from "@/lib/ipc/ai"

const preset: AgentPresetDraft = {
  id: "codex",
  label: "Codex",
  package: "@agentclientprotocol/codex-acp",
  version: "1.12.0",
  command: "npx",
  args: [],
  env: [],
  detected: true,
  launcher: null,
  agentProgram: null,
  signIn: "codex login",
}

const none = {
  providers: [],
  agents: [],
  presets: [],
  presetStates: {},
  models: {},
}

describe("providerRows", () => {
  it("offers every family the form offers, and a custom agent", () => {
    expect(providerRows(none).map((row) => row.id)).toEqual([
      "family:anthropic",
      "family:openai",
      "family:openai_compatible",
      "custom-agent",
    ])
  })

  it("never hides a failure behind a success in the same family", () => {
    const second = { ...remoteProvider, id: "anthropic-2", label: "Personal" }
    const [anthropic] = providerRows({
      ...none,
      providers: [remoteProvider, second],
      models: {
        [remoteProvider.id]: { status: "ready", models: [] },
        [second.id]: { status: "error", message: "401" },
      },
    })
    expect(anthropic?.status.label).toBe("Connection failed")
  })

  it("names the declared default model, not one of its own", () => {
    const [anthropic] = providerRows({ ...none, providers: [remoteProvider] })
    expect(anthropic?.line).toBe("Work account · claude-sonnet-5")
    expect(anthropic?.status.label).toBe("Declared")
  })

  it("says an agent without its launcher is not found, not merely absent", () => {
    const [codex] = providerRows({
      ...none,
      presets: [preset],
      presetStates: { codex: { status: "ready", draft: preset } },
    })
    expect(codex?.status.label).toBe("Not found on this machine")
  })

  it("files an agent declared by hand under the custom row", () => {
    const rows = providerRows({
      ...none,
      agents: [{ ...externalAgent, preset: null, label: "Mine" }],
    })
    const custom = rows.find((row) => row.id === "custom-agent")
    expect(custom?.declared).toBe(true)
    expect(custom?.line).toBe("Mine")
  })
})
