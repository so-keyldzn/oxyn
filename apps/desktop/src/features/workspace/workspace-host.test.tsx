import * as React from "react"
import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { OpenConnection } from "@/lib/ipc/types"

const calls = vi.hoisted(() => ({
  mounted: vi.fn(),
  exit: vi.fn(),
  disconnect: vi.fn(() => Promise.resolve()),
  close: vi.fn(() => Promise.resolve()),
  closeConversation: vi.fn(() => Promise.resolve()),
  closeSessions: vi.fn(),
  pathname: "/workspace",
}))

// Only `disconnect` exists here: a `connect`, a `reconnect` or an execution
// on the way back to A would throw instead of passing unseen.
vi.mock("@/lib/ipc/client", () => ({
  backend: { disconnect: calls.disconnect },
  newCommandId: () => "command",
}))
// Its schemas import the mocked `consoles` module; nothing here moves a tab.
vi.mock("@/lib/ipc/windows", () => ({
  windows: { openInNewWindow: vi.fn(), reportConsoles: vi.fn() },
}))
vi.mock("@/lib/ipc/consoles", () => ({ consoles: { close: calls.close } }))
vi.mock("@/lib/ipc/library", () => ({
  library: { listDocuments: () => Promise.resolve({ entries: [] }) },
}))
vi.mock("@/features/assistant/conversation-store", () => ({
  closeConversation: calls.closeConversation,
}))
vi.mock("@/features/metadata/use-preview", () => ({
  previews: { closeSessions: calls.closeSessions },
}))
vi.mock("@/features/workspace/aside-panels", () => ({
  useAsidePanels: () => [],
}))
vi.mock("@tanstack/react-router", () => ({
  CatchBoundary: ({ children }: { children: React.ReactNode }) => children,
  useNavigate: () => () => Promise.resolve(),
  useRouterState: ({
    select,
  }: {
    select: (state: { location: { pathname: string } }) => string
  }) => select({ location: { pathname: calls.pathname } }),
}))
// The screen stands for everything a workspace holds — consoles, results,
// undo history: kept while it stays mounted, lost if it mounts again.
vi.mock("@/features/workspace/workspace-screen", async () => {
  const { useEffect, useImperativeHandle } = await import("react")
  return {
    WorkspaceScreen: ({
      open,
      visible,
      exitRef,
    }: {
      open: OpenConnection
      visible: boolean
      exitRef: React.Ref<() => Promise<{ sessions: Array<string> }>>
    }) => {
      useEffect(() => calls.mounted(open.connection), [open.connection])
      useImperativeHandle(
        exitRef,
        () => () => {
          calls.exit(open.connection)
          return Promise.resolve({ sessions: [open.session] })
        },
        [open]
      )
      return (
        <div
          data-testid={`workspace-${open.connection}`}
          data-visible={String(visible)}
        />
      )
    },
  }
})

const { WorkspaceHost } = await import("./workspace-host")
const { closeConnection, openConnection, session, showConnection } =
  await import("@/features/session")

const opened = (connection: string) =>
  ({ connection, session: `${connection}-catalog` }) as OpenConnection

beforeEach(() =>
  session.setState((state) => ({ ...state, open: null, workspaces: [] }))
)
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("the workspace host", () => {
  it("brings A back after B as it was: same workspace, no command sent", async () => {
    act(() => openConnection(opened("a")))
    render(<WorkspaceHost />)
    act(() => openConnection(opened("b")))
    await screen.findByTestId("workspace-b")
    expect(screen.getByTestId("workspace-a").dataset.visible).toBe("false")

    act(() => {
      showConnection("a")
    })
    expect(screen.getByTestId("workspace-a").dataset.visible).toBe("true")
    expect(screen.getByTestId("workspace-b").dataset.visible).toBe("false")
    // Mounted once each: nothing A held was rebuilt.
    expect(calls.mounted.mock.calls).toEqual([["a"], ["b"]])
    expect(calls.exit).not.toHaveBeenCalled()
    expect(calls.disconnect).not.toHaveBeenCalled()
    expect(calls.close).not.toHaveBeenCalled()
    // A's previews stay its own: none is evicted on the way (#86).
    expect(calls.closeSessions).not.toHaveBeenCalled()
  })

  it("releases a connection disconnected explicitly, drafts first", async () => {
    act(() => openConnection(opened("a")))
    render(<WorkspaceHost />)
    act(() => openConnection(opened("b")))
    await screen.findByTestId("workspace-b")

    act(() => closeConnection("a"))
    await waitFor(() => expect(screen.queryByTestId("workspace-a")).toBeNull())
    expect(calls.exit).toHaveBeenCalledWith("a")
    expect(calls.disconnect).toHaveBeenCalledWith("a")
    expect(calls.closeConversation).toHaveBeenCalledWith("a")
    expect(calls.exit.mock.invocationCallOrder[0]).toBeLessThan(
      calls.disconnect.mock.invocationCallOrder[0] ?? 0
    )
    // B, hidden or not, is untouched.
    expect(screen.getByTestId("workspace-b")).toBeTruthy()
    expect(calls.disconnect).not.toHaveBeenCalledWith("b")
  })
})
