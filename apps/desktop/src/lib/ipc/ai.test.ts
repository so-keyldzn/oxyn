import { describe, expect, it } from "vitest"

import {
  AgentOptionCategory,
  AgentSettingAnswer,
  AiUpdate,
  Ending,
  FailureCategory,
  SampleRequest,
} from "./ai"

// `finished` and `failed` are the only way a turn leaves its running state, and
// nothing awaits them: `guarded` drops what it cannot read. Refusing one would
// leave the assistant spinning and the thread unable to take another question
// — a worse failure than the mis-drawn badge validation replaced. ADR-0031.

describe("what ends a turn", () => {
  it("degrades an unknown ending instead of refusing the event", () => {
    expect(Ending.parse({ type: "quotaExhausted", turns: 3 })).toEqual({
      type: "unknown",
    })
  })

  it("still reads the endings it knows", () => {
    expect(
      Ending.parse({ type: "answered", turns: 2, truncated: false, cut: null })
    ).toEqual({ type: "answered", turns: 2, truncated: false, cut: null })
  })

  it("degrades an unknown failure category", () => {
    expect(FailureCategory.parse("quotaExceeded")).toBe("unknown")
    expect(FailureCategory.parse("refused")).toBe("refused")
  })

  it("carries a whole `failed` event through, category included", () => {
    const update = AiUpdate.parse({
      node: 4,
      event: {
        kind: "failed",
        message: "the provider said no",
        category: "somethingNewInRust",
        retryable: false,
        signIn: null,
        foundElsewhere: null,
        exit: null,
      },
    })
    expect(update.event).toMatchObject({
      kind: "failed",
      category: "unknown",
    })
  })
})

describe("an agent's settings", () => {
  it("never reads a category it does not know as a known one", () => {
    // The effort selector reads `thoughtLevel` only: a guessed category would
    // show a control for a setting the agent never offered.
    expect(AgentOptionCategory.parse("reasoningBudget")).toBe("other")
    expect(AgentOptionCategory.parse("thoughtLevel")).toBe("thoughtLevel")
  })

  it("reads what Rust sends", () => {
    // The JSON asserted by `an_agents_settings_cross_as_the_front_reads_them`.
    const update = AiUpdate.parse({
      node: 1,
      event: JSON.parse(
        '{"kind":"agentSettings","modes":[{"id":"plan","name":"Plan","description":null}],"currentMode":"plan","options":[{"id":"effort","name":"Effort","description":null,"category":"thoughtLevel","value":{"type":"select","current":"high","choices":[{"id":"high","name":"High","description":"Slower"}]}},{"id":"fast","name":"Fast","description":null,"category":"other","value":{"type":"boolean","on":false}}]}'
      ),
    })
    expect(update.event).toMatchObject({
      kind: "agentSettings",
      currentMode: "plan",
      options: [
        { id: "effort", value: { type: "select", current: "high" } },
        { id: "fast", value: { type: "boolean", on: false } },
      ],
    })
  })
})

describe("a settings change", () => {
  it("reads the refusals Rust sends, and degrades one it does not know", () => {
    // The JSON asserted by `a_refused_change_says_why_and_carries_no_words_of_the_agent`.
    expect(
      AgentSettingAnswer.parse({
        type: "refused",
        reason: "byAgent",
        code: -32000,
      })
    ).toEqual({ type: "refused", reason: "byAgent", code: -32000 })
    // The JSON asserted by the same Rust test.
    expect(
      AgentSettingAnswer.parse({
        type: "sent",
        settings: { modes: [], currentMode: "plan", options: [] },
      })
    ).toEqual({
      type: "sent",
      settings: { modes: [], currentMode: "plan", options: [] },
    })
    expect(
      AgentSettingAnswer.parse({
        type: "refused",
        reason: "somethingNewInRust",
        code: null,
      })
    ).toEqual({ type: "refused", reason: "unknown", code: null })
  })
})

describe("an approved sample", () => {
  it("reads the backend's offer, address included", () => {
    const offer = SampleRequest.parse({
      id: "grant",
      source: "main.customers",
      address: { catalog: null, namespace: "main", relation: "customers" },
      rows: 5,
      fields: [],
      destination: "Local model",
      reach: "local",
    })
    expect(offer.address.relation).toBe("customers")
  })

  it("reads that a sample went, and that the next question starts over", () => {
    const sent = AiUpdate.parse({
      node: 1,
      event: { kind: "sampleApproved", rows: 5, columns: 2 },
    })
    expect(sent.event).toEqual({ kind: "sampleApproved", rows: 5, columns: 2 })
    const reset = AiUpdate.parse({
      node: 2,
      event: { kind: "memoryReset", reason: "sampleNotKept" },
    })
    expect(reset.event).toEqual({
      kind: "memoryReset",
      reason: "sampleNotKept",
    })
  })
})
