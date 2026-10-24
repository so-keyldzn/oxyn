import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AssistantPanel } from "./assistant-panel"
import type * as ConversationStore from "./conversation-store"
import { enabledEntry } from "@/components/oxyn/assistant-fixtures"
import { WorkspaceLayout } from "@/components/oxyn/workspace-layout"
import type { SampleRequest } from "@/lib/ipc/ai"
import type { OpenConnection } from "@/lib/ipc/types"

const calls = vi.hoisted(() => ({
  requestSample: vi.fn(),
  askQuestion: vi.fn(() => Promise.resolve()),
  withdrawSample: vi.fn(),
}))

// The panel itself is under test — its sample approval, owned by the send
// that awaits it. What it reaches in the backend is stubbed at the store.
vi.mock("./conversation-store", async (importActual) => {
  const actual = await importActual<typeof ConversationStore>()
  return {
    ...actual,
    useAssistant: () => actual.INITIAL_ASSISTANT,
    resumeAssistant: () => Promise.resolve(),
    requestSample: calls.requestSample,
    askQuestion: calls.askQuestion,
    withdrawSample: calls.withdrawSample,
  }
})
vi.mock("./agent-startup", () => ({
  useAgentStartup: () => ({ state: null, signIn: {} }),
  startAgent: () => Promise.resolve(),
  startAgentAgain: () => Promise.resolve(),
  stopAgentStart: () => Promise.resolve(),
  signInStartedAgent: () => Promise.resolve(),
}))
vi.mock("./use-assistant-available", () => ({
  useAssistantAvailable: () => enabledEntry,
}))
vi.mock("./object-pin", () => ({
  usePinState: () => ({
    chosenKey: null,
    pin: {
      key: "public.clients",
      label: "clients",
      address: { catalog: null, namespace: "public", relation: "clients" },
    },
  }),
  chooseDestination: vi.fn(),
  unpin: vi.fn(),
}))
vi.mock("./use-provider-models", () => ({
  useProviderModels: () => ({
    data: undefined,
    error: null,
    isError: false,
    isPending: true,
    fetchStatus: "idle",
  }),
}))
vi.mock("./queries", () => ({
  orphanThreadsQuery: { queryKey: ["orphans"], queryFn: () => [] },
  prunedHistoryQuery: { queryKey: ["pruned"], queryFn: () => null },
  agentOptionsQuery: () => ({ queryKey: ["agents"], queryFn: () => [] }),
}))
vi.mock("./reload-agents", () => ({
  useReloadAgents: () => ({ reload: vi.fn(), reloading: false, error: null }),
}))
vi.mock("./mention-search", () => ({ useMentionSearch: () => null }))
vi.mock("./semantic-ranking", () => ({ preloadSemanticModel: vi.fn() }))
vi.mock("./tool-rows", () => ({ ToolRows: () => null }))
vi.mock("./erd-block", () => ({ ErdBlock: () => null }))
// The conversation view is presentational and has its own stories: here it
// only sends, and what the panel answers is kept to be watched.
const asked = vi.hoisted(() => ({ result: null as Promise<boolean> | null }))
vi.mock("@/components/oxyn/assistant-view", () => ({
  AssistantView: ({
    onAsk,
  }: {
    onAsk: (question: string, mentions: Array<never>) => Promise<boolean>
  }) => (
    <button
      type="button"
      onClick={() => {
        asked.result = onAsk("Which clients churned?", [])
      }}
    >
      Ask
    </button>
  ),
}))

// The side column and the resizable group measure themselves; jsdom cannot.
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
)
vi.stubGlobal("matchMedia", (query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addEventListener: () => undefined,
  removeEventListener: () => undefined,
  addListener: () => undefined,
  removeListener: () => undefined,
  dispatchEvent: () => false,
}))

const open: OpenConnection = {
  connection: "018f0000-0000-7000-8000-000000000001",
  session: "018f0000-0000-7000-8000-000000000002",
  name: "billing primary",
  driver: "postgresql",
  environment: "production",
  readOnly: false,
  privacyTier: "sampled",
  capabilities: ["SQL"],
  console: {
    session: "018f0000-0000-7000-8000-000000000003",
    capabilities: [],
    readOnly: false,
    transactionState: "unknown",
  },
}

const request: SampleRequest = {
  id: "approval-1",
  requestedBy: null,
  source: "public.clients",
  address: { catalog: null, namespace: "public", relation: "clients" },
  rows: 12,
  destination: "Claude Sonnet",
  reach: "remote",
  fields: [
    {
      name: "email",
      position: 0,
      logicalType: "text",
      rawType: "text",
      nullable: true,
      default: null,
      comment: null,
      primaryKey: false,
      inferred: false,
    },
  ],
}

function Workspace({ compact }: { compact: boolean }) {
  return (
    <WorkspaceLayout
      sidebar={null}
      sidebarOpen
      onSidebarOpenChange={() => undefined}
      leftView="catalog"
      onLeftViewChange={() => undefined}
      connectionName={open.name}
      environment={open.environment}
      readOnly={false}
      tabs={null}
      activeTab={null}
      onActiveTabChange={() => undefined}
      aside={<AssistantPanel open={open} onOpenInConsole={() => undefined} />}
      asideOpen
      onAsideOpenChange={() => undefined}
      compact={compact}
      statusBar={null}
    >
      {null}
    </WorkspaceLayout>
  )
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  asked.result = null
})

describe("the assistant panel across the compact threshold", () => {
  it("keeps a pending sample approval, and its send, in both directions", async () => {
    calls.requestSample.mockResolvedValue(request)
    const client = new QueryClient()
    const view = (compact: boolean) => (
      <QueryClientProvider client={client}>
        <Workspace compact={compact} />
      </QueryClientProvider>
    )
    const { rerender } = render(view(false))

    fireEvent.click(screen.getByRole("button", { name: "Ask" }))
    const dialog = await screen.findByRole("alertdialog")
    let settled = false
    void asked.result?.then(() => {
      settled = true
    })

    // Wide to compact, then back: the same screen, still awaited by the
    // same send — never closed under the user, never answered for them.
    for (const compact of [true, false]) {
      act(() => rerender(view(compact)))
      expect(screen.getByRole("alertdialog")).toBe(dialog)
      await act(() => Promise.resolve())
      expect(settled).toBe(false)
    }
    expect(calls.withdrawSample).not.toHaveBeenCalled()

    // The decision taken after both moves reaches the send that asked.
    fireEvent.click(screen.getByRole("checkbox", { name: /email/ }))
    fireEvent.click(screen.getByRole("button", { name: /Send 1 of 1/ }))
    await expect(asked.result).resolves.toBe(true)
    expect(calls.askQuestion).toHaveBeenCalledWith(
      open.connection,
      expect.anything(),
      "Which clients churned?",
      { request: "approval-1", source: request.address, columns: ["email"] },
      []
    )
  })
})
