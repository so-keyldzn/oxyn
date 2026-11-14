import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { QueryClientProvider } from "@tanstack/react-query"
import { afterEach, describe, expect, it, vi } from "vitest"
import { createQueryClient } from "@/lib/query-client"
import { backend } from "@/lib/ipc/client"
import type { OpenConnection } from "@/lib/ipc/types"
import { session, openConnection } from "@/features/session"
import { ExplorerProvider, ExplorerNavigation } from "./explorer-provider"

const opened = (connection: string) =>
  ({
    connection,
    session: `${connection}-catalog`,
    name: connection,
    driver: "postgres",
    environment: "development",
    readOnly: true,
    console: { session: `${connection}-console` },
  }) as OpenConnection

afterEach(() => {
  cleanup()
  session.setState((state) => ({ ...state, open: null, workspaces: [] }))
  vi.restoreAllMocks()
})

describe("the window explorer integration", () => {
  it("keeps selection separate, coalesces an opening, and retains the later active workspace", async () => {
    let deliver!: (open: OpenConnection) => void
    const reconnect = vi.spyOn(backend, "reconnect").mockReturnValue(
      new Promise((resolve) => {
        deliver = resolve
      })
    )
    const cancel = vi.spyOn(backend, "cancel").mockResolvedValue(false)
    const client = createQueryClient()
    client.setDefaultOptions({
      queries: { ...client.getDefaultOptions().queries, staleTime: Infinity },
    })
    client.setQueryData(["server-profiles"], [])
    client.setQueryData(["drivers"], [])
    client.setQueryData(
      ["connections"],
      ["a", "b", "c"].map((id) => ({
        id,
        name: id,
        location: `localhost / ${id}`,
        environment: "development",
        readOnly: true,
      }))
    )
    openConnection(opened("a"))
    openConnection(opened("b"))
    render(
      <QueryClientProvider client={client}>
        <ExplorerProvider>
          <ExplorerNavigation />
        </ExplorerProvider>
      </QueryClientProvider>
    )
    const row = (id: string) =>
      screen.getByRole("button", {
        name: new RegExp(`^${id}.*Saved destination`),
      })
    fireEvent.click(row("a"))
    expect(session.state.open?.connection).toBe("b")
    expect(reconnect).not.toHaveBeenCalled()
    fireEvent.doubleClick(row("c"))
    fireEvent.doubleClick(row("c"))
    await waitFor(() => expect(reconnect).toHaveBeenCalledTimes(1))
    fireEvent.keyDown(row("a"), { key: "Enter" })
    expect(session.state.open?.connection).toBe("a")
    await act(async () => {
      deliver(opened("c"))
    })
    expect(session.state.open?.connection).toBe("a")
    expect(session.state.workspaces.map((open) => open.connection)).toEqual([
      "a",
      "b",
      "c",
    ])
    expect(cancel).not.toHaveBeenCalled()
    client.clear()
  })
})
