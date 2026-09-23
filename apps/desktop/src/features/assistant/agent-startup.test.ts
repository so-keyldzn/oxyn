import { beforeEach, describe, expect, it, vi } from "vitest"

import {
  getAgentStartup,
  signInStartedAgent,
  startAgent,
  startAgentAgain,
  stopAgentStart,
  updateStartedSettings,
} from "./agent-startup"
import type { AgentStart, AgentStartRequest } from "@/lib/ipc/ai"

const backend = vi.hoisted(() => ({
  starts: [] as Array<AgentStartRequest>,
  stops: [] as Array<string>,
  signIns: [] as Array<[string, string | null, string]>,
  answers: [] as Array<(answer: AgentStart) => void>,
}))

vi.mock("@/lib/ipc/ai", () => ({
  ai: {
    startAgent: (asked: AgentStartRequest) => {
      backend.starts.push(asked)
      return new Promise<AgentStart>((resolve) => backend.answers.push(resolve))
    },
    stopAgentStart: (connection: string) => {
      backend.stops.push(connection)
      return Promise.resolve(true)
    },
    authenticate: (
      connection: string,
      thread: string | null,
      method: string
    ) => {
      backend.signIns.push([connection, thread, method])
      return Promise.resolve(null)
    },
  },
}))

const settings = { modes: [], currentMode: "plan", options: [] }

function request(connection: string, parent: number | null = null) {
  return {
    connection,
    session: "session-1",
    thread: null,
    parent,
    agent: "agent-0badf00d",
  }
}

function answer(value: AgentStart) {
  const resolve = backend.answers.shift()
  if (!resolve) throw new Error("no start is waiting")
  resolve(value)
}

beforeEach(() => {
  backend.starts = []
  backend.stops = []
  backend.signIns = []
  backend.answers = []
})

describe("starting an agent before its first question", () => {
  it("offers what the agent declared, and starts one agent per question", async () => {
    const done = startAgent("c-1", request("c-1"), "Claude Code")
    // A second render asks for the same start: nothing more is launched.
    void startAgent("c-1", request("c-1"), "Claude Code")
    expect(getAgentStartup("c-1").status).toBe("starting")
    answer({ state: "ready", version: "Claude Agent 0.78.0", settings })
    await done
    expect(backend.starts).toHaveLength(1)
    expect(getAgentStartup("c-1")).toMatchObject({
      status: "ready",
      version: "Claude Agent 0.78.0",
      settings: { currentMode: "plan" },
    })
  })

  it("never starts again on its own after a failure (I-13)", async () => {
    const done = startAgent("c-2", request("c-2"), "Claude Code")
    answer({
      state: "failed",
      message: "Claude Code did not finish starting within 120 seconds",
      category: "agentTimedOut",
      signIn: null,
      foundElsewhere: null,
      exit: null,
    })
    await done
    await startAgent("c-2", request("c-2"), "Claude Code")
    expect(backend.starts).toHaveLength(1)
    expect(getAgentStartup("c-2").status).toBe("failed")

    // The user's click does.
    startAgentAgain("c-2")
    expect(backend.starts).toHaveLength(2)
    expect(getAgentStartup("c-2").status).toBe("starting")
  })

  it("says a refusal in the backend's words", async () => {
    const done = startAgent("c-3", request("c-3"), "Claude Code")
    const resolve = backend.answers.shift()
    expect(resolve).toBeDefined()
    // A rejected call, as `call` rejects on an `IpcError`.
    resolve?.(
      Promise.reject(new Error("This connection is local-only")) as never
    )
    await done
    expect(getAgentStartup("c-3")).toMatchObject({
      status: "failed",
      failure: {
        category: "refused",
        message: "This connection is local-only",
      },
    })
  })

  it("stops on Cancel, keeps saying so, and ignores the late answer", async () => {
    const done = startAgent("c-4", request("c-4"), "Claude Code")
    await stopAgentStart("c-4", true)
    expect(backend.stops).toEqual(["c-4"])
    answer({ state: "cancelled" })
    await done
    expect(getAgentStartup("c-4").status).toBe("stopped")
    // Stopped is not started again by a re-render of the same question.
    await startAgent("c-4", request("c-4"), "Claude Code")
    expect(backend.starts).toHaveLength(1)
  })

  it("stops without a trace when the panel moves to someone else", async () => {
    const done = startAgent("c-5", request("c-5"), "Claude Code")
    answer({ state: "ready", version: null, settings })
    await done
    await stopAgentStart("c-5")
    expect(getAgentStartup("c-5").status).toBe("idle")
    // Nothing started: nothing to stop, and nothing asked of the backend.
    await stopAgentStart("c-6")
    expect(backend.stops).toEqual(["c-5"])
  })

  it("starts again for the next question, which the backend may answer at once", async () => {
    const first = startAgent("c-7", request("c-7"), "Claude Code")
    answer({ state: "ready", version: null, settings })
    await first
    void startAgent("c-7", request("c-7", 0), "Claude Code")
    expect(backend.starts.map((start) => start.parent)).toEqual([null, 0])
  })

  it("shows the settings the agent answered a change with", async () => {
    const done = startAgent("c-8", request("c-8"), "Claude Code")
    answer({ state: "ready", version: null, settings })
    await done
    updateStartedSettings("c-8", { ...settings, currentMode: "default" })
    expect(getAgentStartup("c-8")).toMatchObject({
      settings: { currentMode: "default" },
    })
  })

  it("signs in through the started agent, then starts it again", async () => {
    const done = startAgent("c-9", request("c-9"), "Codex")
    answer({
      state: "failed",
      message: "the agent needs you to sign in before it can answer",
      category: "agentSignIn",
      signIn: { agent: "Codex", methods: [], terminalCommand: "codex login" },
      foundElsewhere: null,
      exit: null,
    })
    await done
    await signInStartedAgent("c-9", "chat-gpt")
    // No conversation yet: the agent started for the connection is asked.
    expect(backend.signIns).toEqual([["c-9", null, "chat-gpt"]])
    expect(backend.starts).toHaveLength(2)
  })
})
