import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { DestinationOption } from "./availability"
import { useProviderModels } from "./use-provider-models"

const asked = vi.hoisted(() => ({ providers: [] as Array<string> }))

vi.mock("@/lib/ipc/ai", () => ({
  ai: {
    providerModels: (provider: string) => {
      asked.providers.push(provider)
      return Promise.resolve([
        {
          id: "claude-opus-5",
          displayName: "Claude Opus 5",
          contextWindow: null,
          cost: null,
          reasoningEfforts: ["low", "high"],
        },
      ])
    },
  },
}))

const provider = (usable: boolean): DestinationOption => ({
  key: "provider:anthropic-0000abcd",
  kind: "provider",
  id: "anthropic-0000abcd",
  label: "Work",
  model: "claude-opus-5",
  reach: "remote",
  usable,
  reason: usable ? null : "refused by the tier",
})

function withClient(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

const client = () =>
  new QueryClient({ defaultOptions: { queries: { retry: false } } })

describe("the selected provider's models", () => {
  beforeEach(() => {
    asked.providers = []
  })

  it("are asked for when the panel shows the provider, without a menu opening", async () => {
    const { result } = renderHook(() => useProviderModels(provider(true)), {
      wrapper: withClient(client()),
    })
    await waitFor(() =>
      expect(result.current.data?.[0]?.reasoningEfforts).toEqual([
        "low",
        "high",
      ])
    )
    expect(asked.providers).toEqual(["anthropic-0000abcd"])
  })

  it("are asked for once per provider, not at every opening of the panel", async () => {
    const shared = client()
    const first = renderHook(() => useProviderModels(provider(true)), {
      wrapper: withClient(shared),
    })
    await waitFor(() => expect(first.result.current.data).toBeDefined())
    first.unmount()
    const again = renderHook(() => useProviderModels(provider(true)), {
      wrapper: withClient(shared),
    })
    await waitFor(() => expect(again.result.current.data).toBeDefined())
    expect(asked.providers).toEqual(["anthropic-0000abcd"])
  })

  it("are not asked for an agent, nothing selected, or a provider the tier refuses", async () => {
    const agent: DestinationOption = {
      ...provider(true),
      key: "agent:x",
      kind: "agent",
    }
    for (const selected of [agent, null, provider(false)]) {
      renderHook(() => useProviderModels(selected), {
        wrapper: withClient(client()),
      })
    }
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(asked.providers).toEqual([])
  })
})
