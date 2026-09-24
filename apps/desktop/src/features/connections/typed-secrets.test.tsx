import * as React from "react"
import {
  QueryClient,
  QueryClientProvider,
  useMutation,
} from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ConnectionDraft } from "@/lib/ipc/types"
import { useTypedSecrets } from "./typed-secrets"
import type { WithoutSecrets } from "./typed-secrets"

// A password chosen to be unmistakable if it ever leaks into JSON.
const SENTINEL = "s3cr3t-sentinel"

const draft = (name: string): ConnectionDraft => ({
  driver: "postgresql",
  name,
  environment: "development",
  privacyTier: "metadata",
  readOnly: false,
  values: { host: "localhost" },
  secrets: { password: SENTINEL },
})

function withClient(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

/**
 * Renders `useTypedSecrets` alongside a `useMutation` wired exactly like the
 * screens: `mutationFn` reassembles the draft from `take()` inside its own
 * call, never from a variable captured outside it.
 */
function setUp(
  client: QueryClient,
  send: (draft: ConnectionDraft) => Promise<{ name: string }>
) {
  return renderHook(
    () => {
      const { hold, take } = useTypedSecrets()
      const mutation = useMutation({
        mutationFn: (withoutSecrets: WithoutSecrets<ConnectionDraft>) =>
          send({ ...withoutSecrets, secrets: take(withoutSecrets) }),
      })
      return { hold, take, mutation }
    },
    { wrapper: withClient(client) }
  )
}

function noSecretLeaked(client: QueryClient) {
  const mutations = client.getMutationCache().getAll()
  const serialized = JSON.stringify(mutations.map((mutation) => mutation.state))
  expect(serialized).not.toContain(SENTINEL)
}

describe("useTypedSecrets", () => {
  it("hands the secret to the mutation function but keeps it out of the settled cache (success)", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    })
    const send = vi.fn(async (received: ConnectionDraft) => ({
      name: received.name,
    }))
    const { result } = setUp(client, send)

    const variables = result.current.hold(draft("primary"))
    act(() => {
      result.current.mutation.mutate(variables)
    })

    await waitFor(() => expect(result.current.mutation.isSuccess).toBe(true))

    expect(send).toHaveBeenCalledTimes(1)
    expect(send.mock.calls[0]?.[0]?.secrets).toEqual({ password: SENTINEL })

    const mutations = client.getMutationCache().getAll()
    expect(mutations).toHaveLength(1)
    expect(
      (mutations[0]?.state.variables as { name?: string } | undefined)?.name
    ).toBe("primary")

    noSecretLeaked(client)
    // `mutationFn` already consumed the secrets; nothing is left to leak.
    expect(result.current.take(variables)).toEqual({})
  })

  it("gives each attempt its own secrets on a double submit", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    })
    const send = vi.fn(async (received: ConnectionDraft) => ({
      name: received.name,
    }))
    const { result } = setUp(client, send)

    // Both `hold` calls happen before either `mutationFn` runs, as on a
    // double click: a single slot would hand the first attempt the second
    // draft's secrets and the second attempt none at all.
    const first = result.current.hold({
      ...draft("first"),
      secrets: { password: "first-password" },
    })
    const second = result.current.hold({
      ...draft("second"),
      secrets: { password: "second-password" },
    })
    act(() => {
      result.current.mutation.mutate(first)
      result.current.mutation.mutate(second)
    })

    await waitFor(() => expect(send).toHaveBeenCalledTimes(2))

    const sent = new Map(
      send.mock.calls.map(([received]) => [received.name, received.secrets])
    )
    expect(sent.get("first")).toEqual({ password: "first-password" })
    expect(sent.get("second")).toEqual({ password: "second-password" })
  })

  it("keeps the secret out of the cache even when the mutation fails and stays observed", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    })
    // Typed explicitly, not inferred from a zero-argument arrow: otherwise
    // `send.mock.calls[0]?.[0]` would type as `never` instead of
    // `ConnectionDraft`, and the assertion below could not fail.
    const send = vi.fn(
      async (_received: ConnectionDraft): Promise<{ name: string }> => {
        throw new Error("wrong password")
      }
    )
    const { result } = setUp(client, send)

    act(() => {
      result.current.mutation.mutate(result.current.hold(draft("second")))
    })

    await waitFor(() => expect(result.current.mutation.isError).toBe(true))

    expect(send).toHaveBeenCalledTimes(1)
    expect(send.mock.calls[0]?.[0]?.secrets).toEqual({ password: SENTINEL })

    // The mutation is still mounted (`mutation.error` drives the alert shown
    // to the user) — the case `gcTime` cannot shorten.
    const mutations = client.getMutationCache().getAll()
    expect(mutations).toHaveLength(1)
    expect(
      (mutations[0]?.state.variables as { name?: string } | undefined)?.name
    ).toBe("second")

    noSecretLeaked(client)
  })
})
