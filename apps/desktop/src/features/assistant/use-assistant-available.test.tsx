import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import {
  NO_LANGUAGE,
  NO_USABLE_DESTINATION,
  UNREADABLE,
  assistantEntry,
  defaultDestination,
  noAgentFor,
  readingAgentsFor,
} from "./availability"
import { useAssistantAvailable } from "./use-assistant-available"
import type { AgentOption, DeclaredProvider, ExternalAgent } from "@/lib/ipc/ai"

const declared = vi.hoisted(() => ({
  providers: [] as Array<DeclaredProvider>,
  agents: [] as Array<ExternalAgent>,
  roles: [] as Array<AgentOption>,
  listed: [] as Array<string>,
}))

vi.mock("@/lib/ipc/ai", () => ({
  ai: {
    providers: () => Promise.resolve(declared.providers),
    externalAgents: () => Promise.resolve(declared.agents),
    listAgents: (connection: string) => {
      declared.listed.push(connection)
      return Promise.resolve(declared.roles)
    },
  },
}))

/** An agent role as `ai_list_agents` returns it. */
function role(name: string, error: string | null = null): AgentOption {
  return {
    id: `id-${name}`,
    name,
    description: "",
    origin: "user",
    error,
    disabledDestinations: [],
  }
}

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
    declared.roles = []
    declared.listed = []
  })

  it("draws no AI entry at all without a declaration", async () => {
    const { result } = renderHook(
      () =>
        useAssistantAvailable({
          connection: "c1",
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
          connection: "c1",
          privacyTier: "metadata",
          capabilities: ["SQL"],
        }),
      { wrapper }
    )
    await waitFor(() => expect(result.current.status).toBe("enabled"))
    // A SQL session never waits for the list of agents.
    expect(declared.listed).toEqual([])
  })

  it("opens on a Redis session once an agent writing Redis is listed", async () => {
    declared.providers = [remote]
    declared.roles = [role("Keys")]
    const { result } = renderHook(
      () =>
        useAssistantAvailable({
          connection: "c2",
          privacyTier: "metadata",
          capabilities: ["REDIS_COMMAND"],
        }),
      { wrapper }
    )
    await waitFor(() => expect(result.current.status).toBe("enabled"))
    expect(declared.listed).toEqual(["c2"])
  })

  it("stays closed on a Redis session no agent writes", async () => {
    declared.providers = [remote]
    declared.roles = [role("broken.md", "broken.md, line 1: …")]
    const { result } = renderHook(
      () =>
        useAssistantAvailable({
          connection: "c3",
          privacyTier: "metadata",
          capabilities: ["REDIS_COMMAND"],
        }),
      { wrapper }
    )
    await waitFor(() =>
      expect(result.current).toMatchObject({
        status: "disabled",
        reason: noAgentFor("redis"),
      })
    )
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

  it("says so when the session declares no single language", () => {
    expect(
      assistantEntry({ ...base, capabilities: [], tier: "metadata" })
    ).toMatchObject({ status: "disabled", reason: NO_LANGUAGE })
  })

  it("names the language no agent writes yet", () => {
    const entry = assistantEntry({
      ...base,
      capabilities: ["REDIS_COMMAND"],
      tier: "metadata",
      roles: [],
    })
    expect(entry).toMatchObject({
      status: "disabled",
      reason: noAgentFor("redis"),
    })
    if (entry.status !== "disabled") return
    expect(entry.reason).toContain("Redis commands")
  })

  it("waits for the agents of a language before saying none writes it", () => {
    expect(
      assistantEntry({
        ...base,
        capabilities: ["MONGO_QUERY"],
        tier: "metadata",
      })
    ).toMatchObject({
      status: "disabled",
      reason: readingAgentsFor("mongo"),
    })
  })

  it("opens for a language once an agent that is not in error writes it", () => {
    const entry = (roles: Array<AgentOption>) =>
      assistantEntry({
        ...base,
        capabilities: ["SEARCH_DSL"],
        tier: "metadata",
        roles,
      })
    expect(entry([role("Search")]).status).toBe("enabled")
    expect(entry([role("broken.md", "broken.md, line 2: …")])).toMatchObject({
      status: "disabled",
      reason: noAgentFor("search-dsl"),
    })
    // The tier still decides afterwards.
    expect(
      assistantEntry({
        ...base,
        agents: [],
        capabilities: ["SEARCH_DSL"],
        tier: "local",
        roles: [role("Search")],
      })
    ).toMatchObject({ status: "disabled", reason: NO_USABLE_DESTINATION })
  })

  it("never needs the list of agents on a SQL session", () => {
    expect(
      assistantEntry({ ...base, tier: "metadata", roles: undefined }).status
    ).toBe("enabled")
  })

  it("prefers a provider to an agent", () => {
    const entry = assistantEntry({ ...base, tier: "metadata" })
    if (entry.status === "absent") throw new Error("expected destinations")
    expect(defaultDestination(entry.destinations)?.kind).toBe("provider")
  })
})
