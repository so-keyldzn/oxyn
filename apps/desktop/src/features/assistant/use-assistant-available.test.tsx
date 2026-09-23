import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import {
  NO_SQL,
  NO_USABLE_DESTINATION,
  UNREADABLE,
  assistantEntry,
  defaultDestination,
} from "./availability"
import { useAssistantAvailable } from "./use-assistant-available"
import type { DeclaredProvider, ExternalAgent } from "@/lib/ipc/ai"

const declared = vi.hoisted(() => ({
  providers: [] as Array<DeclaredProvider>,
  agents: [] as Array<ExternalAgent>,
}))

vi.mock("@/lib/ipc/ai", () => ({
  ai: {
    providers: () => Promise.resolve(declared.providers),
    externalAgents: () => Promise.resolve(declared.agents),
  },
}))

const remote: DeclaredProvider = {
  id: "anthropic-0000abcd",
  label: "Work",
  kind: "anthropic",
  endpoint: "https://api.anthropic.com",
  endpointRedacted: false,
  model: "claude-sonnet-5",
  keyConfigured: true,
  reach: "remote",
  measuredAtMs: 0,
}

const agent: ExternalAgent = {
  id: "agent-0000abcd",
  label: "Claude Code",
  command: "claude",
  argCount: 1,
  envNames: [],
  preset: null,
  confined: false,
}

function wrapper({ children }: { children: React.ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

describe("useAssistantAvailable", () => {
  beforeEach(() => {
    declared.providers = []
    declared.agents = []
  })

  it("draws no AI entry at all without a declaration", async () => {
    const { result } = renderHook(
      () =>
        useAssistantAvailable({
          privacyTier: "metadata",
          capabilities: ["SQL"],
        }),
      { wrapper }
    )
    // Absent while reading, and still absent once the empty list arrived.
    expect(result.current).toEqual({ status: "absent" })
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(result.current).toEqual({ status: "absent" })
  })

  it("appears once a provider is declared", async () => {
    declared.providers = [remote]
    const { result } = renderHook(
      () =>
        useAssistantAvailable({
          privacyTier: "metadata",
          capabilities: ["SQL"],
        }),
      { wrapper }
    )
    await waitFor(() => expect(result.current.status).toBe("enabled"))
  })
})

describe("assistantEntry", () => {
  const base = {
    providers: [remote],
    agents: [agent],
    unreadable: false,
    capabilities: ["SQL"],
  }

  it("stays visible and explains itself when the tier refuses everything", () => {
    const entry = assistantEntry({ ...base, tier: "local" })
    expect(entry).toMatchObject({
      status: "disabled",
      reason: NO_USABLE_DESTINATION,
    })
  })

  it("admits a local provider on a local connection, never an agent", () => {
    const local = {
      ...remote,
      id: "openai_compatible-1",
      reach: "local" as const,
    }
    const entry = assistantEntry({
      ...base,
      providers: [remote, local],
      tier: "local",
    })
    expect(entry.status).toBe("enabled")
    if (entry.status === "absent") return
    expect(defaultDestination(entry.destinations)?.id).toBe(local.id)
    expect(
      entry.destinations.find((option) => option.kind === "agent")?.usable
    ).toBe(false)
  })

  it("never rounds an unresolved endpoint to local", () => {
    const unresolved = { ...remote, reach: "unresolved" as const }
    const entry = assistantEntry({
      ...base,
      providers: [unresolved],
      agents: [],
      tier: "local",
    })
    expect(entry.status).toBe("disabled")
  })

  it("does not read an unreadable list as an empty one", () => {
    expect(
      assistantEntry({
        ...base,
        providers: undefined,
        unreadable: true,
        tier: "metadata",
      })
    ).toMatchObject({ status: "disabled", reason: UNREADABLE })
  })

  it("says so when the session has no SQL", () => {
    expect(
      assistantEntry({ ...base, capabilities: [], tier: "metadata" })
    ).toMatchObject({ status: "disabled", reason: NO_SQL })
  })

  it("prefers a provider to an agent", () => {
    const entry = assistantEntry({ ...base, tier: "metadata" })
    if (entry.status === "absent") throw new Error("expected destinations")
    expect(defaultDestination(entry.destinations)?.kind).toBe("provider")
  })
})
