import { describe, expect, it } from "vitest"

import {
  isLocalEndpoint,
  modelEntries,
  probeOf,
  reasonCopy,
} from "./model-picker-model"
import type { ModelChoice, ModelListingReason } from "@/lib/ipc/ai"

function model(id: string, displayName = id): ModelChoice {
  return {
    id,
    displayName,
    contextWindow: null,
    cost: null,
    reasoningEfforts: [],
  }
}

const listed = [
  model("gpt-alpha", "Alpha"),
  model("gpt-beta", "Beta"),
  model("gpt-gamma", "Gamma"),
]

describe("modelEntries", () => {
  it("keeps the provider's order, the current value first", () => {
    expect(
      modelEntries(listed, "gpt-gamma", "", "OpenAI").map((entry) => entry.id)
    ).toEqual(["gpt-gamma", "gpt-alpha", "gpt-beta"])
  })

  it("filters on the id, the name and the provider label", () => {
    const ids = (query: string) =>
      modelEntries(listed, "", query, "OpenAI")
        .filter((entry) => entry.origin === "listed")
        .map((entry) => entry.id)
    expect(ids("BETA")).toEqual(["gpt-beta"])
    expect(ids("gamma")).toEqual(["gpt-gamma"])
    expect(ids("openai")).toHaveLength(3)
  })

  it("flags a current value the provider no longer lists", () => {
    const [first] = modelEntries(listed, "gpt-retired", "", "OpenAI")
    expect(first).toEqual({
      id: "gpt-retired",
      label: "gpt-retired",
      origin: "current",
      unavailable: true,
    })
  })

  it("does not call a value unavailable before a list answered", () => {
    expect(
      modelEntries(null, "gpt-retired", "", "OpenAI")[0]?.unavailable
    ).toBe(false)
  })

  it("offers the typed id unless a listed model carries it", () => {
    expect(modelEntries(listed, "", "my-deployment", "OpenAI")).toEqual([
      {
        id: "my-deployment",
        label: "my-deployment",
        origin: "manual",
        unavailable: false,
      },
    ])
    expect(
      modelEntries(listed, "", "gpt-beta", "OpenAI").some(
        (entry) => entry.origin === "manual"
      )
    ).toBe(false)
  })
})

describe("probeOf", () => {
  const fresh = {
    kind: "openai" as const,
    endpoint: "https://api.openai.com/v1",
    key: "",
    editing: null,
  }

  it("waits for a key on a remote endpoint", () => {
    expect(probeOf(fresh, true)).toBeNull()
    expect(probeOf({ ...fresh, key: "sk-test" }, true)).toEqual({
      id: null,
      kind: "openai",
      baseUrl: "https://api.openai.com/v1",
      key: "sk-test",
    })
  })

  it("needs no key on a local endpoint", () => {
    expect(
      probeOf({ ...fresh, endpoint: "http://127.0.0.1:11434/v1" }, true)
    ).not.toBeNull()
  })

  it("reuses a stored key only for the stored endpoint", () => {
    const editing = {
      id: "openai-1",
      endpoint: "https://api.openai.com/v1",
      keyConfigured: true,
    }
    expect(probeOf({ ...fresh, endpoint: "", editing }, true)).toEqual({
      id: "openai-1",
      kind: "openai",
      baseUrl: "",
      key: null,
    })
    expect(
      probeOf(
        { ...fresh, endpoint: "https://elsewhere.test/v1", editing },
        true
      )
    ).toBeNull()
  })

  it("lets a click ask without a key, and never without an endpoint", () => {
    expect(probeOf(fresh, false)).not.toBeNull()
    expect(probeOf({ ...fresh, endpoint: " " }, false)).toBeNull()
  })
})

describe("isLocalEndpoint", () => {
  it("reads the host, not the path", () => {
    expect(isLocalEndpoint("http://localhost:11434/v1")).toBe(true)
    expect(isLocalEndpoint("http://[::1]:8080")).toBe(true)
    expect(isLocalEndpoint("https://example.test/localhost")).toBe(false)
  })
})

describe("reasonCopy", () => {
  it("reads a reason this build does not know generically", () => {
    expect(reasonCopy("quotaExceeded" as ModelListingReason)).toBe(
      "The models could not be listed."
    )
    expect(reasonCopy("unauthorized")).toMatch(/refused/)
  })
})
