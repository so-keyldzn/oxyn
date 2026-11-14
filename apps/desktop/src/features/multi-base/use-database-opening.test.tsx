import * as React from "react"
import { act, cleanup, renderHook } from "@testing-library/react"
import { QueryClientProvider } from "@tanstack/react-query"
import { afterEach, describe, expect, it, vi } from "vitest"
import { postgresDriver, savedConnections } from "@/components/oxyn/fixtures"
import { createQueryClient } from "@/lib/query-client"
import { settingsBackend } from "@/lib/ipc/settings"
import { serverProfiles } from "@/lib/ipc/server-profiles"
import type { ServerProfileSnapshot } from "@/lib/ipc/server-profiles"
import type { OpenConnection, ConnectionDraft } from "@/lib/ipc/types"
import { WorkspaceActivations } from "./activation"
import { useDatabaseOpening } from "./use-database-opening"

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})
const source = {
  connection: "source",
  session: "source-catalog",
} as OpenConnection
const group: ServerProfileSnapshot = {
  profile: {
    id: "example-group",
    name: "Example",
    driver: "postgres",
    params: { host: "localhost" },
  },
  revision: 1,
  connections: [],
}
const detail = {
  ...savedConnections[0]!,
  driver: "postgres",
  values: { host: "localhost", database: "source", user: "reader" },
  hasStoredSecrets: true,
}
const draft: ConnectionDraft = {
  name: "Child",
  driver: "postgres",
  environment: "development",
  privacyTier: "metadata",
  readOnly: true,
  values: {
    host: "localhost",
    port: "5432",
    database: "child",
    user: "analyst",
  },
  secrets: {},
}

function setup() {
  vi.spyOn(settingsBackend, "connectionDetails").mockResolvedValue(detail)
  const reconnect = vi.fn(
    async (_command: string, connection: string) =>
      ({ connection }) as OpenConnection
  )
  const activations = new WorkspaceActivations({
    command: () => "example-command",
    held: () => false,
    show: vi.fn(),
    capacity: () => true,
    reconnect,
    adopt: vi.fn(),
    discard: vi.fn(async () => {}),
    cancel: vi.fn(async () => {}),
    changed: vi.fn(),
  })
  const client = createQueryClient()
  const hook = renderHook(
    () => useDatabaseOpening(activations, vi.fn(), vi.fn()),
    {
      wrapper: ({ children }: { children: React.ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    }
  )
  return { hook, activations, reconnect }
}
describe("independent child connection recovery", () => {
  it("keeps the saved identity after an authentication failure and never resubmits creation", async () => {
    const { hook, reconnect } = setup()
    const save = vi
      .spyOn(serverProfiles, "open")
      .mockResolvedValue({
        type: "saved",
        connection: { ...savedConnections[0]!, id: "saved-child" },
        message: "Synthetic authentication failure",
        retryable: false,
      })
    await act(() =>
      hook.result.current.choose(group, source, "child", postgresDriver, true)
    )
    await act(() => hook.result.current.submit(draft))
    expect(hook.result.current.wizard?.saved).toBe(true)
    await act(() => hook.result.current.submit(draft))
    expect(save).toHaveBeenCalledTimes(1)
    expect(save.mock.calls[0]?.[3].values).toEqual({
      host: "localhost",
      database: "child",
      user: "analyst",
    })
    act(() => hook.result.current.close())
    vi.mocked(settingsBackend.connectionDetails).mockImplementation(
      async (id) =>
        id === "saved-child"
          ? { ...detail, id, values: { ...detail.values, database: "child" } }
          : detail
    )
    await act(() =>
      hook.result.current.choose(group, source, "child", postgresDriver, false)
    )
    expect(reconnect).toHaveBeenCalledExactlyOnceWith(
      "example-command",
      "saved-child"
    )
    expect(save).toHaveBeenCalledTimes(1)
  })
  it("a cancellation before dispatch leaves the form usable without sending credentials", async () => {
    const { hook } = setup()
    const save = vi.spyOn(serverProfiles, "open")
    await act(() =>
      hook.result.current.choose(group, source, "child", postgresDriver, true)
    )
    await act(async () => {
      const pending = hook.result.current.submit(draft)
      await hook.result.current.cancel()
      await pending
    })
    expect(save).not.toHaveBeenCalled()
    expect(hook.result.current.wizard?.submitting).toBe(false)
  })
  it("asks about distinct accounts rather than merging matching database names", async () => {
    const { hook, reconnect } = setup()
    vi.mocked(settingsBackend.connectionDetails).mockImplementation(
      async (id) =>
        id === "source"
          ? detail
          : {
              ...detail,
              id,
              values: { ...detail.values, database: "child", user: id },
            }
    )
    await act(() =>
      hook.result.current.choose(
        { ...group, connections: ["reader", "writer"] },
        source,
        "child",
        postgresDriver,
        false
      )
    )
    expect(
      hook.result.current.wizard?.candidates.map((candidate) => candidate.id)
    ).toEqual(["reader", "writer"])
    expect(reconnect).not.toHaveBeenCalled()
  })
})
