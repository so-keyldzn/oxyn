import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ProviderSettingsViewProps } from "@/components/oxyn/provider-settings-view"
import type { AgentDraft, ProviderDraft } from "@/lib/ipc/ai"

const calls = vi.hoisted(() => ({
  saveProvider: vi.fn(),
  saveExternalAgent: vi.fn(),
  view: null as ProviderSettingsViewProps | null,
}))

vi.mock("./queries", () => ({
  aiKeys: {
    all: ["ai"],
    detected: (id: string) => ["ai", "detected", id],
  },
  providersQuery: {
    queryKey: ["ai", "providers"],
    queryFn: () => Promise.resolve([]),
  },
  agentsQuery: {
    queryKey: ["ai", "agents"],
    queryFn: () => Promise.resolve([]),
  },
  presetsQuery: {
    queryKey: ["ai", "presets"],
    queryFn: () => Promise.resolve([]),
  },
  detectedAgentQuery: (id: string) => ({
    queryKey: ["ai", "detected", id],
    queryFn: () => Promise.resolve(null),
  }),
  modelsQuery: (id: string) => ({
    queryKey: ["ai", "models", id],
    queryFn: () => Promise.resolve([]),
  }),
}))
vi.mock("@/lib/ipc/ai", () => ({
  ai: {
    saveProvider: calls.saveProvider,
    saveExternalAgent: calls.saveExternalAgent,
    removeProvider: vi.fn(),
    removeExternalAgent: vi.fn(),
    listDraftModels: vi.fn(),
  },
}))
vi.mock("@/components/oxyn/provider-settings-view", () => ({
  ProviderSettingsView: (props: ProviderSettingsViewProps) => {
    calls.view = props
    return null
  },
}))
vi.mock("@/components/ui/toast", () => ({ toast: { add: vi.fn() } }))

const { ProviderSettings } = await import("./provider-settings")

const provider: ProviderDraft = {
  id: null,
  kind: "openai",
  label: "Work",
  baseUrl: "https://api.example.test",
  model: "model-1",
  key: "provider-key-sentinel",
  clearKey: false,
}

const agent: AgentDraft = {
  id: null,
  label: "Local agent",
  command: "agent",
  args: [],
  env: [
    { name: "MODE", value: "review" },
    { name: "API_KEY", value: "agent-secret-sentinel", secret: true },
  ],
}

function view() {
  if (!calls.view) throw new Error("provider settings view was not rendered")
  return calls.view
}

function cacheText(client: QueryClient) {
  return JSON.stringify(
    client
      .getMutationCache()
      .getAll()
      .map((mutation) => mutation.state.variables)
  )
}

describe("provider setting secrets", () => {
  beforeEach(() => {
    calls.saveProvider.mockReset()
    calls.saveExternalAgent.mockReset()
    calls.view = null
  })

  it("keeps provider keys and secret agent environment values out of mutation variables", async () => {
    calls.saveProvider.mockResolvedValueOnce({})
    calls.saveExternalAgent.mockResolvedValueOnce({ id: "agent-1" })
    const client = new QueryClient({
      defaultOptions: {
        mutations: { retry: false },
        queries: { retry: false },
      },
    })
    render(
      <QueryClientProvider client={client}>
        <ProviderSettings />
      </QueryClientProvider>
    )
    await waitFor(() => expect(calls.view).not.toBeNull())

    await act(async () => {
      await view().onSaveProvider(provider)
      await view().onSaveAgent(agent)
    })

    expect(calls.saveProvider).toHaveBeenCalledWith(provider)
    expect(calls.saveExternalAgent).toHaveBeenCalledWith(agent)
    expect(cacheText(client)).not.toContain("provider-key-sentinel")
    expect(cacheText(client)).not.toContain("agent-secret-sentinel")
  })

  it("keeps a provider key out of the observed cache after a failed save", async () => {
    calls.saveProvider.mockRejectedValueOnce(
      new Error("provider is unavailable")
    )
    const client = new QueryClient({
      defaultOptions: {
        mutations: { retry: false },
        queries: { retry: false },
      },
    })
    render(
      <QueryClientProvider client={client}>
        <ProviderSettings />
      </QueryClientProvider>
    )
    await waitFor(() => expect(calls.view).not.toBeNull())

    let saved: boolean | undefined
    await act(async () => {
      saved = await view().onSaveProvider(provider)
    })

    expect(saved).toBe(false)
    expect(calls.saveProvider).toHaveBeenCalledWith(provider)
    expect(cacheText(client)).not.toContain("provider-key-sentinel")
  })

  it("keeps each double-submit provider key out of mutation variables", async () => {
    calls.saveProvider.mockResolvedValueOnce({}).mockResolvedValueOnce({})
    const client = new QueryClient({
      defaultOptions: {
        mutations: { retry: false },
        queries: { retry: false },
      },
    })
    render(
      <QueryClientProvider client={client}>
        <ProviderSettings />
      </QueryClientProvider>
    )
    await waitFor(() => expect(calls.view).not.toBeNull())

    await act(async () => {
      await Promise.all([
        view().onSaveProvider({
          ...provider,
          label: "First",
          key: "first-provider-key-sentinel",
        }),
        view().onSaveProvider({
          ...provider,
          label: "Second",
          key: "second-provider-key-sentinel",
        }),
      ])
    })

    expect(calls.saveProvider).toHaveBeenCalledWith({
      ...provider,
      label: "First",
      key: "first-provider-key-sentinel",
    })
    expect(calls.saveProvider).toHaveBeenCalledWith({
      ...provider,
      label: "Second",
      key: "second-provider-key-sentinel",
    })
    expect(cacheText(client)).not.toContain("first-provider-key-sentinel")
    expect(cacheText(client)).not.toContain("second-provider-key-sentinel")
  })
})
