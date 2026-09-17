import { describe, expect, it } from "vitest"

import { destinationChoice } from "./availability"
import type { DestinationOption } from "./availability"
import { declaredEfforts, effortToSend } from "./reasoning-effort"
import { ModelChoice } from "@/lib/ipc/ai"

const model = (id: string, reasoningEfforts: Array<string>) =>
  ModelChoice.parse({
    id,
    displayName: id,
    contextWindow: null,
    cost: null,
    reasoningEfforts,
  })

const MODELS = [model("opus", ["low", "high", "max"]), model("haiku", [])]

describe("the reasoning effort a question carries", () => {
  it("is the chosen one when the model declares it", () => {
    expect(effortToSend("max", MODELS, "opus")).toBe("max")
  })

  it("is none for a model that does not declare it", () => {
    // Chosen on one model, then the model changed: not carried over.
    expect(effortToSend("max", MODELS, "haiku")).toBeNull()
    expect(effortToSend("medium", MODELS, "opus")).toBeNull()
  })

  it("is none before the model list is read, or for an unlisted model", () => {
    expect(effortToSend("high", null, "opus")).toBeNull()
    expect(effortToSend("high", MODELS, "sonnet")).toBeNull()
  })

  it("is none when nothing was chosen", () => {
    expect(effortToSend(null, MODELS, "opus")).toBeNull()
  })
})

describe("the efforts a model declares", () => {
  it("drops a level this build does not know instead of failing the list", () => {
    expect(model("next", ["low", "ultra", "max"]).reasoningEfforts).toEqual([
      "low",
      "max",
    ])
  })

  it("is empty for a model with none — no selector", () => {
    expect(declaredEfforts(MODELS, "haiku")).toEqual([])
  })
})

describe("where the effort travels", () => {
  const option = (kind: "provider" | "agent"): DestinationOption => ({
    key: `${kind}:x`,
    kind,
    id: "x",
    label: "X",
    model: null,
    reach: "local",
    usable: true,
    reason: null,
  })

  it("goes with a provider's question, and never with an agent's", () => {
    expect(destinationChoice(option("provider"), null, "low")).toEqual({
      kind: "provider",
      id: "x",
      model: null,
      effort: "low",
    })
    expect(destinationChoice(option("agent"), null, "low")).toEqual({
      kind: "agent",
      id: "x",
    })
  })
})
