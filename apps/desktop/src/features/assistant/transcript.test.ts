import { describe, expect, it } from "vitest"

import {
  answerText,
  canContinue,
  decisionSettled,
  decisionStarted,
  endingLine,
  emptyExchange,
  exchangeOf,
  outcomeOf,
  pendingApprovals,
  reduce,
  saidSomething,
  touched,
} from "./transcript"
import type { AiEvent } from "@/lib/ipc/ai"

const call = (id: number, mutating = false): AiEvent => ({
  kind: "toolCall",
  call: id,
  tool: "execute_query",
  command: "Execute",
  statement: "UPDATE t SET a = 1",
  connection: "commerce",
  environment: "staging",
  mutating,
})

const failed = (
  category: "refused" | "provider" | "agentSignIn",
  retryable = category !== "refused"
): AiEvent => ({
  kind: "failed",
  message: "nope",
  category,
  retryable,
  signIn: null,
  foundElsewhere: null,
})

describe("an exchange", () => {
  it("merges streamed text and splits it around a tool call", () => {
    const exchange = exchangeOf("q", [
      { kind: "textDelta", text: "Let me " },
      { kind: "textDelta", text: "look." },
      call(0),
      { kind: "textDelta", text: "Done." },
    ])
    expect(answerText(exchange)).toBe("Let me look.\n\nDone.")
    expect(exchange.entries.map((entry) => entry.kind)).toEqual([
      "answer",
      "tool",
      "answer",
    ])
  })

  it("times reasoning once, and says when it was hidden", () => {
    const exchange = exchangeOf("q", [
      { kind: "thinkingDelta", text: "The user" },
      { kind: "thinkingDelta", text: " wants…" },
      { kind: "thinkingRedacted" },
      { kind: "thinkingEnded", elapsedMs: 4200 },
      { kind: "textDelta", text: "Here." },
    ])
    expect(exchange.entries[0]).toMatchObject({
      kind: "thinking",
      text: "The user wants…",
      redacted: true,
      elapsedMs: 4200,
    })
  })

  it("turns the oldest draft into the call, and drops drafts left at the end", () => {
    const drafting = exchangeOf("q", [
      { kind: "toolDraft", index: 0, tool: "execute_query" },
      { kind: "toolArguments", index: 0, fragment: '{"sql":"SEL' },
      { kind: "toolArguments", index: 0, fragment: 'ECT 1"}' },
    ])
    expect(drafting.entries[0]).toMatchObject({
      kind: "toolDraft",
      arguments: '{"sql":"SELECT 1"}',
    })
    const called = exchangeOf("q", [
      { kind: "toolDraft", index: 0, tool: "execute_query" },
      call(0),
    ])
    expect(called.entries.map((entry) => entry.kind)).toEqual(["tool"])
    const abandoned = exchangeOf("q", [
      { kind: "toolDraft", index: 0, tool: "execute_query" },
      { kind: "finished", ending: { type: "cancelled", turns: 1 } },
    ])
    expect(abandoned.entries.map((entry) => entry.kind)).toEqual(["ended"])
  })

  it("keeps an approval open while the model goes on, and a question approves nothing", () => {
    const exchange = exchangeOf("Mark invoice paid", [
      call(0, true),
      {
        kind: "approvalRequested",
        call: 0,
        approval: "cmd-1",
        reason: "review",
        statement: "UPDATE t SET a = 1",
        connection: "commerce",
        environment: "staging",
        actor: "agent",
      },
      {
        kind: "toolReported",
        call: 0,
        status: "awaitingApproval",
        detail: "review",
        errorClass: null,
        withheld: false,
        rows: null,
      },
      { kind: "textDelta", text: "Waiting for you." },
      {
        kind: "finished",
        ending: { type: "answered", turns: 1, truncated: false, cut: null },
      },
    ])
    expect(pendingApprovals(exchange)).toHaveLength(1)
    const deciding = decisionStarted(exchange, "cmd-1")
    expect(pendingApprovals(deciding)[0]?.state).toBe("deciding")
    const rejected = decisionSettled(deciding, "cmd-1", false, {
      type: "denied",
      reason: "",
    } as never)
    expect(rejected.entries[0]).toMatchObject({
      state: "rejected",
      approval: null,
    })
  })

  it("distinguishes rejected, denied and cancelled tool calls", () => {
    const denied = exchangeOf("q", [
      call(0, true),
      {
        kind: "toolReported",
        call: 0,
        status: "denied",
        detail: "agents may not write on production",
        errorClass: null,
        withheld: false,
        rows: null,
      },
    ])
    expect(denied.entries[0]).toMatchObject({ state: "denied" })
    const cancelled = exchangeOf("q", [
      call(0),
      {
        kind: "toolReported",
        call: 0,
        status: "cancelled",
        detail: "cancelled",
        errorClass: "ambiguous",
        withheld: false,
        rows: null,
      },
    ])
    expect(cancelled.entries[0]).toMatchObject({ state: "cancelled" })
  })

  it("offers to ask again exactly when the backend says so, whatever the category", () => {
    // A provider failure after a write is not retryable: only the backend
    // knows a write ran. The category must not override it.
    expect(
      outcomeOf(exchangeOf("q", [failed("provider", false)]))
    ).toMatchObject({ retryable: false })
    expect(
      outcomeOf(exchangeOf("q", [failed("agentSignIn", true)]))
    ).toMatchObject({ retryable: true })
  })

  it("adds usage up and keeps an undeclared cache unknown, not zero", () => {
    const exchange = exchangeOf("q", [
      {
        kind: "usage",
        input: 1000,
        output: 20,
        cacheRead: null,
        cacheWrite: null,
      },
      {
        kind: "usage",
        input: 1200,
        output: 300,
        cacheRead: 900,
        cacheWrite: null,
      },
    ])
    expect(exchange.usage).toEqual({
      input: 2200,
      output: 320,
      cacheRead: 900,
      cacheWrite: null,
      turns: 2,
    })
  })

  it("updates an agent tool in place", () => {
    const exchange = exchangeOf("q", [
      { kind: "agentTool", id: "t1", tool: "read", status: "pending" },
      { kind: "agentTool", id: "t1", tool: null, status: "failed" },
      { kind: "permissionRefused", action: "read", reason: "no file system" },
    ])
    expect(exchange.entries).toMatchObject([
      { kind: "agentTool", tool: "read", status: "failed" },
      { kind: "permissionRefused" },
    ])
    expect(saidSomething(exchange)).toBe(true)
  })

  it("offers to continue a cut, paused or limited answer only", () => {
    const ended = (ending: AiEvent) => canContinue(exchangeOf("q", [ending]))
    expect(
      ended({
        kind: "finished",
        ending: {
          type: "answered",
          turns: 1,
          truncated: true,
          cut: "tokenLimit",
        },
      })
    ).toBe(true)
    expect(
      ended({ kind: "finished", ending: { type: "paused", turns: 1 } })
    ).toBe(true)
    expect(
      ended({
        kind: "finished",
        ending: { type: "answered", turns: 1, truncated: false, cut: null },
      })
    ).toBe(false)
    expect(
      ended({ kind: "finished", ending: { type: "refused", turns: 1 } })
    ).toBe(false)
    expect(canContinue(emptyExchange("q"))).toBe(false)
  })
})

describe("an agent's settings", () => {
  const settings = (
    currentMode: string | null,
    ...options: Array<[string, string]>
  ): AiEvent => ({
    kind: "agentSettings",
    modes: [{ id: "ask", name: "Ask", description: null }],
    currentMode,
    options: options.map(([id, current]) => ({
      id,
      name: id,
      description: null,
      category: "thoughtLevel" as const,
      value: { type: "select" as const, current, choices: [] },
    })),
  })

  it("replaces the previous ones instead of merging them", () => {
    // An option the agent dropped must leave the panel: a selector for a
    // setting the agent no longer offers does nothing when used.
    const first = reduce(
      emptyExchange("q"),
      settings("ask", ["effort", "high"], ["model", "a"])
    )
    const second = reduce(first, settings("plan", ["effort", "low"]))

    expect(second.agentSettings?.currentMode).toBe("plan")
    expect(
      second.agentSettings?.options.map((option) => [
        option.id,
        option.value.type === "select" ? option.value.current : null,
      ])
    ).toEqual([["effort", "low"]])
  })

  it("is absent until an agent declares some", () => {
    // `null`: no selector at all, rather than an empty one.
    expect(emptyExchange("q").agentSettings).toBeNull()
  })
})

describe("an agent's plan", () => {
  const plan = (...steps: Array<string>): AiEvent => ({
    kind: "plan",
    entries: steps.map((content) => ({
      content,
      priority: "medium" as const,
      status: "pending" as const,
    })),
  })

  it("replaces the previous one instead of merging it", () => {
    // The mistake one writes spontaneously: fusing by position or by content.
    // The protocol sends the whole plan on every change and entries have no
    // identifier, so a merge keeps steps the agent has dropped — the plan grows
    // at every turn and nothing fails.
    const first = reduce(emptyExchange("q"), plan("Read the schema", "Guess"))
    const second = reduce(first, plan("Read the schema", "Write the query"))

    expect(second.plan?.map((entry) => entry.content)).toEqual([
      "Read the schema",
      "Write the query",
    ])
  })

  it("distinguishes no plan from an empty one", () => {
    // `null` means the agent sent none; `[]` means it sent one and it was
    // empty. The panel shows a waiting line for the first and nothing for the
    // second, so the two must not collapse.
    expect(emptyExchange("q").plan).toBeNull()
    expect(reduce(emptyExchange("q"), plan()).plan).toEqual([])
  })

  it("keeps the state each step is in", () => {
    const exchange = reduce(emptyExchange("q"), {
      kind: "plan",
      entries: [
        { content: "Read", priority: "high", status: "completed" },
        { content: "Write", priority: "low", status: "inProgress" },
      ],
    })
    expect(exchange.plan).toEqual([
      { content: "Read", priority: "high", status: "completed" },
      { content: "Write", priority: "low", status: "inProgress" },
    ])
  })
})

describe("what an answer touched", () => {
  const reported = (
    id: number,
    status: "completed" | "denied" | "failed" | "cancelled",
    rows: number | null,
    detail = "1 rows, 1 batches"
  ): AiEvent => ({
    kind: "toolReported",
    call: id,
    status,
    detail,
    errorClass: null,
    withheld: false,
    rows,
  })

  it("cites completed commands only, by connection name, in order", () => {
    const exchange = exchangeOf("q", [
      call(0),
      reported(0, "completed", 42),
      call(1, true),
      reported(1, "denied", null, "agents may not write on production"),
      call(2),
      reported(2, "failed", null, "relation does not exist"),
      call(3),
      reported(3, "cancelled", null),
    ])
    expect(touched(exchange)).toEqual([
      {
        tool: "execute_query",
        command: "Execute",
        connection: "commerce",
        mutating: false,
        rows: 42,
      },
    ])
  })

  it("takes the count from the structured field, never from the words", () => {
    // The detail says one row; the executor measured none. The words are for
    // the user to read, and a rewording must not change a citation.
    const exchange = exchangeOf("q", [
      call(0),
      reported(0, "completed", null, "1 rows, 1 batches"),
    ])
    expect(touched(exchange)[0]?.rows).toBeNull()
  })

  it("keeps the count of an approved run, and none for a cancelled one", () => {
    const asked = exchangeOf("q", [
      call(0, true),
      {
        kind: "approvalRequested",
        call: 0,
        approval: "cmd-1",
        reason: "review",
        statement: "UPDATE t SET a = 1",
        connection: "commerce",
        environment: "staging",
        actor: "agent",
      },
    ])
    const executed = (cancelled: boolean) =>
      decisionSettled(decisionStarted(asked, "cmd-1"), "cmd-1", true, {
        type: "executed",
        result: "r",
        columns: [],
        rows: 3,
        elapsedMs: 1,
        complete: true,
        cancelled,
        truncated: false,
      })
    expect(touched(executed(false))).toMatchObject([
      { mutating: true, rows: 3 },
    ])
    expect(touched(executed(true))).toEqual([])
  })
})

describe("an ending line", () => {
  const cut = (
    reason: "tokenLimit" | "contextWindow" | "providerError" | "unknown"
  ) => endingLine({ type: "answered", turns: 2, truncated: true, cut: reason })

  it("names the token limit only when the token limit cut the answer", () => {
    expect(cut("tokenLimit")).toContain("token limit")
    for (const other of [
      "contextWindow",
      "providerError",
      "unknown",
    ] as const) {
      expect(cut(other)).not.toContain("by the token limit")
    }
    expect(cut("contextWindow")).toContain("context window")
    expect(cut("providerError")).toContain("reported an error")
  })
})

describe("an approved sample", () => {
  it("leaves counts in the transcript, never what was sent", () => {
    const exchange = reduce(emptyExchange("q"), {
      kind: "sampleApproved",
      rows: 5,
      columns: 2,
    })
    expect(exchange.entries).toEqual([
      { kind: "sampleSent", key: "s-0", rows: 5, columns: 2 },
    ])
  })
})
