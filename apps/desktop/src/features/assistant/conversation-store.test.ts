import { beforeEach, describe, expect, it, vi } from "vitest"

import {
  askQuestion,
  changeAgentSetting,
  closeConversation,
  getAssistant,
  newThread,
  openThread,
  resumeAssistant,
  withdrawSample,
} from "./conversation-store"
import { activePath } from "./thread"
import type {
  AgentSettingsState,
  AiUpdate,
  AskStarted,
  ThreadSummary,
  ThreadView,
} from "@/lib/ipc/ai"

const backend = vi.hoisted(() => ({
  ask: vi.fn(),
  cancel: vi.fn(async () => true),
  threads: vi.fn(async (): Promise<Array<ThreadSummary>> => []),
  openThread: vi.fn(),
  forget: vi.fn(async () => undefined),
  selectVersion: vi.fn(async () => undefined),
  setAgentSetting: vi.fn(),
  withdrawSample: vi.fn(),
}))

vi.mock("@/lib/ipc/ai", () => ({ ai: backend }))
vi.mock("@/lib/ipc/client", () => ({
  backend: { decide: vi.fn() },
  BackendError: class extends Error {},
}))

const target = {
  session: "session-1",
  destination: {
    kind: "provider" as const,
    id: "p1",
    model: null,
    effort: null,
  },
}

const answered: AiUpdate = {
  node: 0,
  event: {
    kind: "finished",
    ending: { type: "answered", turns: 1, truncated: false, cut: null },
  },
}

let connections = 0
function connection() {
  connections += 1
  return `connection-${connections}`
}

beforeEach(() => {
  vi.clearAllMocks()
  backend.threads.mockResolvedValue([])
})

describe("an approved sample", () => {
  const approval = {
    request: "grant",
    source: {
      catalog: null,
      namespace: "main",
      relation: "customers",
    },
    columns: ["name"],
  }

  it("goes with the question it was approved for, and not with the next one", async () => {
    const id = connection()
    const captured: { stream?: (update: AiUpdate) => void } = {}
    let node = 0
    backend.ask.mockImplementation(
      async (_request: unknown, onUpdate: (update: AiUpdate) => void) => {
        captured.stream = onUpdate
        return { thread: "7", node: node++ } satisfies AskStarted
      }
    )

    await askQuestion(id, target, "first", approval)
    captured.stream?.(answered)
    await askQuestion(id, target, "second")
    expect(backend.ask.mock.calls[0]?.[0]).toMatchObject({ sample: approval })
    expect(backend.ask.mock.calls[1]?.[0]).toMatchObject({ sample: null })
  })

  it("is never queued behind a running answer", async () => {
    const id = connection()
    backend.ask.mockResolvedValue({ thread: "7", node: 0 })

    await askQuestion(id, target, "first")
    await expect(
      askQuestion(id, target, "with rows", approval)
    ).rejects.toThrow(/sample/)
    expect(getAssistant(id).thread.queue).toHaveLength(0)
    expect(backend.ask).toHaveBeenCalledTimes(1)
  })
})

describe("the assistant store", () => {
  it("keeps events that arrive before the question is acknowledged", async () => {
    const id = connection()
    const captured: { stream?: (update: AiUpdate) => void } = {}
    backend.ask.mockImplementation(
      async (_request: unknown, onUpdate: (update: AiUpdate) => void) => {
        captured.stream = onUpdate
        // The backend streams before its own answer reaches the webview.
        onUpdate({ node: 0, event: { kind: "textDelta", text: "1,204." } })
        return { thread: "7", node: 0 } satisfies AskStarted
      }
    )

    await askQuestion(id, target, "How many clients?")
    expect(captured.stream).toBeDefined()
    const path = activePath(getAssistant(id).thread)
    expect(path).toHaveLength(1)
    expect(path[0]?.exchange.entries).toMatchObject([
      { kind: "answer", text: "1,204." },
    ])
  })

  it("holds a message typed while an answer runs, and sends it when it ends", async () => {
    const id = connection()
    const captured: { stream?: (update: AiUpdate) => void } = {}
    let node = 0
    backend.ask.mockImplementation(
      async (_request: unknown, onUpdate: (update: AiUpdate) => void) => {
        captured.stream = onUpdate
        return { thread: "7", node: node++ } satisfies AskStarted
      }
    )

    await askQuestion(id, target, "first")
    expect(await askQuestion(id, target, "then this one")).toBe("queued")
    expect(getAssistant(id).thread.queue).toHaveLength(1)
    expect(backend.ask).toHaveBeenCalledTimes(1)

    captured.stream?.(answered)
    await vi.waitFor(() => expect(backend.ask).toHaveBeenCalledTimes(2))
    expect(getAssistant(id).thread.queue).toHaveLength(0)
    // Sent as a follow-up of the answer it waited for.
    expect(backend.ask.mock.calls[1]?.[0]).toMatchObject({
      thread: "7",
      parent: 0,
      question: "then this one",
    })
  })

  it("holds the queue after a failure: the user reads it first", async () => {
    const id = connection()
    const captured: { stream?: (update: AiUpdate) => void } = {}
    backend.ask.mockImplementation(
      async (_request: unknown, onUpdate: (update: AiUpdate) => void) => {
        captured.stream = onUpdate
        return { thread: "8", node: 0 } satisfies AskStarted
      }
    )
    await askQuestion(id, target, "first")
    await askQuestion(id, target, "queued one")

    captured.stream?.({
      node: 0,
      event: {
        kind: "failed",
        message: "529 overloaded",
        category: "provider",
        retryable: true,
        signIn: null,
        foundElsewhere: null,
        exit: null,
      },
    })
    await vi.waitFor(() => expect(getAssistant(id).thread.running).toBeNull())
    expect(backend.ask).toHaveBeenCalledTimes(1)
    expect(getAssistant(id).thread.queue).toHaveLength(1)
  })

  it("drops the answer of a conversation the panel has left", async () => {
    const id = connection()
    const view: ThreadView = {
      id: "3",
      title: "Old one",
      nodes: [{ id: 0, parent: null, question: "q", events: [] }],
      selections: [],
      running: null,
    }
    const gate: { release?: (value: ThreadView) => void } = {}
    backend.openThread.mockImplementation(
      async () =>
        new Promise<ThreadView>((resolve) => {
          gate.release = resolve
        })
    )

    const opening = openThread(id, "3")
    newThread(id)
    gate.release?.(view)
    await opening
    // The late answer belongs to a conversation nobody is looking at.
    expect(getAssistant(id).thread.id).toBeNull()
    expect(getAssistant(id).thread.nodes).toHaveLength(0)
  })

  it("takes back the conversation that was running, once", async () => {
    const id = connection()
    backend.threads.mockResolvedValue([
      {
        id: "1",
        title: "Old",
        createdAtMs: 1,
        updatedAtMs: 2,
        exchanges: 1,
        running: false,
      },
      {
        id: "2",
        title: "Live",
        createdAtMs: 3,
        updatedAtMs: 4,
        exchanges: 1,
        running: true,
      },
    ])
    backend.openThread.mockResolvedValue({
      id: "2",
      title: "Live",
      nodes: [{ id: 0, parent: null, question: "q", events: [] }],
      selections: [],
      running: 0,
    } satisfies ThreadView)

    await resumeAssistant(id)
    expect(backend.openThread).toHaveBeenCalledWith(id, "2", expect.anything())
    expect(getAssistant(id).thread.running).toBe(0)

    await resumeAssistant(id)
    expect(backend.openThread).toHaveBeenCalledTimes(1)
  })

  it("forgets everything when the connection closes", async () => {
    const id = connection()
    backend.ask.mockResolvedValue({ thread: "9", node: 0 } satisfies AskStarted)
    await askQuestion(id, target, "q")
    await closeConversation(id)
    expect(backend.forget).toHaveBeenCalledWith(id)
    expect(getAssistant(id).thread.nodes).toHaveLength(0)
  })
})

function settingsWith(current: string): AgentSettingsState {
  return {
    modes: [],
    currentMode: null,
    options: [
      {
        id: "model",
        name: "Model",
        description: null,
        category: "model",
        value: {
          type: "select",
          current,
          choices: [
            { id: "sonnet", name: "Sonnet", description: null },
            { id: "opus", name: "Opus", description: null },
          ],
        },
      },
    ],
  }
}

/** A conversation with one answered exchange, whose agent declared settings. */
async function conversationWithSettings(id: string) {
  const captured: { stream?: (update: AiUpdate) => void } = {}
  backend.ask.mockImplementation(
    async (_request: unknown, onUpdate: (update: AiUpdate) => void) => {
      captured.stream = onUpdate
      return { thread: "7", node: 0 } satisfies AskStarted
    }
  )
  await askQuestion(id, target, "which model?")
  captured.stream?.({
    node: 0,
    event: { kind: "agentSettings", ...settingsWith("sonnet") },
  })
  captured.stream?.(answered)
  return captured
}

function shownSettings(id: string) {
  return activePath(getAssistant(id).thread).at(-1)?.exchange.agentSettings
}

describe("changing an agent's setting", () => {
  it("shows the settings the agent answered, and a later event replaces them", async () => {
    const id = connection()
    const captured = await conversationWithSettings(id)
    backend.setAgentSetting.mockResolvedValue({
      type: "sent",
      settings: settingsWith("opus"),
    })

    await changeAgentSetting(id, {
      kind: "select",
      option: "model",
      choice: "opus",
    })

    // The command got the flat shape, on this conversation.
    expect(backend.setAgentSetting).toHaveBeenCalledWith(id, "7", {
      option: "model",
      value: "opus",
    })
    // Shown at once, from the reply alone: no event came.
    expect(shownSettings(id)).toStrictEqual(settingsWith("opus"))

    // Whichever arrives last is shown.
    captured.stream?.({
      node: 0,
      event: { kind: "agentSettings", ...settingsWith("sonnet") },
    })
    expect(shownSettings(id)).toStrictEqual(settingsWith("sonnet"))
  })

  it("keeps the shown settings on a refusal, and says why in words", async () => {
    const id = connection()
    await conversationWithSettings(id)
    const before = shownSettings(id)
    backend.setAgentSetting.mockResolvedValue({
      type: "refused",
      reason: "unknownValue",
      code: null,
    })

    await expect(
      changeAgentSetting(id, {
        kind: "select",
        option: "model",
        choice: "gone",
      })
    ).rejects.toThrow("the agent no longer offers this value.")
    // Untouched — the same object, not an equal copy.
    expect(shownSettings(id)).toBe(before)
  })

  it("does not write a reply into a conversation the panel has left", async () => {
    const id = connection()
    await conversationWithSettings(id)
    let reply: (answer: unknown) => void = () => {}
    backend.setAgentSetting.mockImplementation(
      () => new Promise((resolve) => (reply = resolve))
    )
    const pending = changeAgentSetting(id, {
      kind: "select",
      option: "model",
      choice: "opus",
    })

    // Another conversation, with exchanges and settings of its own, is shown
    // by the time the first one's reply comes back.
    newThread(id)
    const other: { stream?: (update: AiUpdate) => void } = {}
    backend.ask.mockImplementation(
      async (_request: unknown, onUpdate: (update: AiUpdate) => void) => {
        other.stream = onUpdate
        return { thread: "8", node: 0 } satisfies AskStarted
      }
    )
    await askQuestion(id, target, "another question")
    other.stream?.({
      node: 0,
      event: { kind: "agentSettings", ...settingsWith("sonnet") },
    })
    const theirs = shownSettings(id)

    reply({ type: "sent", settings: settingsWith("opus") })
    await pending

    expect(getAssistant(id).thread.id).toBe("8")
    expect(shownSettings(id)).toBe(theirs)
  })
})

describe("a declined sample", () => {
  it("withdraws its grant from the backend", () => {
    backend.withdrawSample.mockResolvedValue(undefined)
    withdrawSample("connection-declined", {
      id: "grant",
      source: "main.customers",
      address: { catalog: null, namespace: "main", relation: "customers" },
      rows: 5,
      fields: [],
      destination: "Local model",
      reach: "local",
    })
    expect(backend.withdrawSample).toHaveBeenCalledWith(
      "connection-declined",
      "grant"
    )
  })
})
