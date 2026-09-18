// Mirror of `crates/oxyn-desktop/src/ipc/ai.rs` and `ipc/ai/conversation.rs`,
// and the AI workspace's calls — as executable schemas, so a field renamed on
// one side is named at the boundary on the first call, instead of surfacing as
// an `undefined` three screens away (ADR-0031). The two sides still change in
// the same commit; the difference is that forgetting is no longer silent.
//
// Types are derived with `z.infer`, so a schema and its type cannot drift.
//
// A declaration is written **before** the ones that use it: schemas are values,
// and a `const` read above its own line evaluates to `undefined` at load —
// which the `interface` declarations this file used to hold hid for free.

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { Nothing, call, guarded } from "./client"
import {
  CatalogAddress,
  Environment,
  PrivacyTier,
  RelationField,
} from "./types"

/**
 * A word Rust carries as a `&'static str`, with **no enum behind it**.
 *
 * The values the type lists are what today's code emits, not a contract: a new
 * preset, a new refusal reason or a new error class is an ordinary addition on
 * the Rust side. A `z.enum` would then fail a perfectly valid answer at the
 * boundary — a regression, not a protection. So the schema checks that it is a
 * string, and the type is what narrows it (ADR-0031).
 */
function freeWord<T extends string>(): z.ZodType<T> {
  return z.custom<T>((value) => typeof value === "string", {
    message: "expected a string",
  })
}

// `openai_compatible` is snake_case in the middle of a camelCase boundary: the
// Rust enum carries no `rename_all`, each variant renames itself
// (`oxyn-core/src/ai.rs`). It is the stable name written into a provenance.
export const ProviderKind = z.enum([
  "anthropic",
  "openai",
  "gemini",
  "openai_compatible",
])
export type ProviderKind = z.infer<typeof ProviderKind>

/** `unresolved` counts as remote for every decision, and is shown as such. */
export const ProviderReach = z.enum(["local", "remote", "unresolved"])
export type ProviderReach = z.infer<typeof ProviderReach>

export const DeclaredProvider = z.object({
  /** Opaque: addresses the declaration, never rendered. */
  id: z.string(),
  label: z.string(),
  kind: ProviderKind,
  /** Without user information, query or fragment. */
  endpoint: z.string(),
  endpointRedacted: z.boolean(),
  model: z.string(),
  /** « a key is stored », never where nor what (I-03). */
  keyConfigured: z.boolean(),
  reach: ProviderReach,
  measuredAtMs: z.number().nonnegative(),
})
export type DeclaredProvider = z.infer<typeof DeclaredProvider>

export const AgentPresetId = freeWord<AgentPresetId>()
export type AgentPresetId = "claude-code" | "codex"

export const ExternalAgent = z.object({
  id: z.string(),
  label: z.string(),
  command: z.string(),
  argCount: z.number().int().nonnegative(),
  /** Names only: values never come back. */
  envNames: z.array(z.string()),
  preset: AgentPresetId.nullable(),
})
export type ExternalAgent = z.infer<typeof ExternalAgent>

/** Sent once, front to back. Nothing returns the key. */
export const ProviderDraft = z.object({
  id: z.string().nullable(),
  kind: ProviderKind,
  label: z.string(),
  baseUrl: z.string(),
  model: z.string(),
  key: z.string().nullable(),
  clearKey: z.boolean(),
})
export type ProviderDraft = z.infer<typeof ProviderDraft>

export const EnvVar = z.object({
  name: z.string(),
  value: z.string(),
})
export type EnvVar = z.infer<typeof EnvVar>

export const AgentDraft = z.object({
  label: z.string(),
  command: z.string(),
  args: z.array(z.string()),
  env: z.array(EnvVar),
})
export type AgentDraft = z.infer<typeof AgentDraft>

/** A ready-made declaration to review; nothing is saved until confirmed. */
export const AgentPresetDraft = z.object({
  id: AgentPresetId,
  label: z.string(),
  package: z.string(),
  version: z.string(),
  command: z.string(),
  args: z.array(z.string()),
  env: z.array(EnvVar),
  /** Whether the machine was looked at. */
  detected: z.boolean(),
  launcher: z.string().nullable(),
  agentProgram: z.string().nullable(),
  /** The terminal command that signs in with this agent. */
  signIn: z.string(),
})
export type AgentPresetDraft = z.infer<typeof AgentPresetDraft>

export const ModelCost = z.object({
  inputPerMillion: z.number().nonnegative(),
  outputPerMillion: z.number().nonnegative(),
  currency: z.string(),
})
export type ModelCost = z.infer<typeof ModelCost>

/** A reasoning effort, as the provider writes it. */
export const ReasoningEffort = z.enum(["low", "medium", "high", "xhigh", "max"])
export type ReasoningEffort = z.infer<typeof ReasoningEffort>

export const ModelChoice = z.object({
  id: z.string(),
  displayName: z.string(),
  contextWindow: z.number().int().nonnegative().nullable(),
  /** Published by the provider; `null` when it publishes none. */
  cost: ModelCost.nullable(),
  /**
   * The efforts the provider declares for this model, in order; empty when it
   * declares none. A level this build does not know is left out rather than
   * failing the whole model list: it cannot be offered, and there is no honest
   * word to show it as.
   */
  reasoningEfforts: z
    .array(z.string())
    .transform((levels) =>
      levels.filter(
        (level): level is ReasoningEffort =>
          ReasoningEffort.safeParse(level).success
      )
    ),
})
export type ModelChoice = z.infer<typeof ModelChoice>

export const DestinationChoice = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("provider"),
    id: z.string(),
    model: z.string().nullable(),
    /** Checked by the backend against the model's declared efforts; `null` sends none. */
    effort: ReasoningEffort.nullable(),
  }),
  z.object({ kind: z.literal("agent"), id: z.string() }),
])
export type DestinationChoice = z.infer<typeof DestinationChoice>

/**
 * An offer to send rows of one relation with the next question: names and a
 * bound, never a value. `id` is the backend's one-use grant — sent back with
 * the approval, never rendered.
 */
export const SampleRequest = z.object({
  id: z.string(),
  source: z.string(),
  address: CatalogAddress,
  rows: z.number().int().nonnegative(),
  fields: z.array(RelationField),
  destination: z.string(),
  reach: ProviderReach,
})
export type SampleRequest = z.infer<typeof SampleRequest>

/** What the user ticked. The backend checks every part of it before reading. */
export const SampleApproval = z.object({
  request: z.string(),
  source: CatalogAddress,
  columns: z.array(z.string()),
})
export type SampleApproval = z.infer<typeof SampleApproval>

export const AskRequest = z.object({
  connection: z.string(),
  session: z.string(),
  /** `null` starts a new conversation. */
  thread: z.string().nullable(),
  /** The exchange this follows; an edit or a regeneration passes its parent. */
  parent: z.number().int().nonnegative().nullable(),
  question: z.string(),
  destination: DestinationChoice,
  /** For this question only: the backend does not keep it for the next one. */
  sample: SampleApproval.nullable(),
})
export type AskRequest = z.infer<typeof AskRequest>

export const AskStarted = z.object({
  thread: z.string(),
  node: z.number().int().nonnegative(),
})
export type AskStarted = z.infer<typeof AskStarted>

export const ThreadSummary = z.object({
  id: z.string(),
  title: z.string(),
  createdAtMs: z.number().nonnegative(),
  updatedAtMs: z.number().nonnegative(),
  exchanges: z.number().int().nonnegative(),
  running: z.boolean(),
})
export type ThreadSummary = z.infer<typeof ThreadSummary>

export const Selection = z.object({
  parent: z.number().int().nonnegative().nullable(),
  node: z.number().int().nonnegative(),
})
export type Selection = z.infer<typeof Selection>

export const Destination = z.object({
  kind: freeWord<"provider" | "agent">(),
  label: z.string(),
  model: z.string().nullable(),
  reach: ProviderReach,
  agentVersion: z.string().nullable(),
})
export type Destination = z.infer<typeof Destination>

export const ContextSummary = z.object({
  relations: z.number().int().nonnegative(),
  omittedRelations: z.number().int().nonnegative(),
  droppedSamples: z.number().int().nonnegative(),
  estimatedTokens: z.number().int().nonnegative(),
})
export type ContextSummary = z.infer<typeof ContextSummary>

/** Signs what a conversation proposes (ADR-0023). Ids are opaque. */
export const AgentProvenance = z.object({
  agent: z.string(),
  session: z.string(),
  kind: ProviderKind,
  model: z.string(),
  at: z.string(),
})
export type AgentProvenance = z.infer<typeof AgentProvenance>

export const ToolStatus = z.enum([
  "completed",
  "awaitingApproval",
  "denied",
  "failed",
  "cancelled",
])
export type ToolStatus = z.infer<typeof ToolStatus>

export const AgentToolStatus = z.enum([
  "pending",
  "running",
  "completed",
  "failed",
])
export type AgentToolStatus = z.infer<typeof AgentToolStatus>

/** What cut an answer short. `unknown` is a word of ignorance, hence the fallback. */
export const Cut = z
  .enum(["tokenLimit", "contextWindow", "providerError", "unknown"])
  .catch("unknown")
export type Cut = z.infer<typeof Cut>

export const Ending = z
  .discriminatedUnion("type", [
    z.object({
      type: z.literal("answered"),
      turns: z.number().int().nonnegative(),
      truncated: z.boolean(),
      cut: Cut.nullable(),
    }),
    z.object({
      type: z.literal("cancelled"),
      turns: z.number().int().nonnegative(),
    }),
    z.object({
      type: z.literal("turnLimit"),
      turns: z.number().int().nonnegative(),
    }),
    z.object({
      type: z.literal("refused"),
      turns: z.number().int().nonnegative(),
    }),
    z.object({
      type: z.literal("paused"),
      turns: z.number().int().nonnegative(),
    }),
    z.object({ type: z.literal("agentLimit") }),
    z.object({ type: z.literal("unknown") }),
  ])
  .catch({ type: "unknown" })
export type Ending = z.infer<typeof Ending>

/**
 * The two schemas that end a turn fall back instead of refusing.
 *
 * `finished` and `failed` are the **only** way the assistant leaves its running
 * state (`features/assistant/conversation-store.ts`): `ai_ask` resolves an
 * `AskStarted` and nothing else awaits them. A refused event is dropped by
 * `guarded` — nobody is waiting for it — and the turn then spins forever, with
 * the thread refusing further questions. Validation would have turned a
 * mis-drawn badge into a stuck conversation.
 *
 * So a category this version does not know degrades to `unknown` rather than
 * failing the event. `unknown` is a **front-side** fallback, not a variant Rust
 * emits, and it is listed as not retryable: what we could not read, we do not
 * offer to replay (I-13).
 */
export const FailureCategory = z
  .enum([
    "refused",
    "provider",
    "setup",
    "agentNotFound",
    "agentSignIn",
    "agentIncompatible",
    "agentExited",
    "agent",
    "unknown",
  ])
  .catch("unknown")
export type FailureCategory = z.infer<typeof FailureCategory>

export const SignInMethod = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().nullable(),
  /** `agent`: Oxyn asks the agent. `terminal`: the user runs `command`. */
  kind: freeWord<"agent" | "terminal">(),
  command: z.string().nullable(),
})
export type SignInMethod = z.infer<typeof SignInMethod>

export const SignInHelp = z.object({
  agent: z.string(),
  methods: z.array(SignInMethod),
  terminalCommand: z.string().nullable(),
})
export type SignInHelp = z.infer<typeof SignInHelp>

export const MemoryReset = z.enum([
  "tierChanged",
  "agentRestarted",
  "destinationChanged",
  "sampleNotKept",
  "restarted",
])
export type MemoryReset = z.infer<typeof MemoryReset>

export const ErrorClass = freeWord<ErrorClass>()
export type ErrorClass = "transient" | "permanent" | "ambiguous" | "unknown"

/** An amount and its ISO 4217 currency, as the agent gives them. */
export const Money = z.object({
  amount: z.number().nonnegative(),
  currency: z.string(),
})
export type Money = z.infer<typeof Money>

/**
 * One step of a run, in order. One `finished` or `failed` ends it.
 *
 * A `discriminatedUnion`, never a `z.union`: `textDelta` and `thinkingDelta`
 * arrive once per token of the model, and this is the most frequent message of
 * the boundary. The tag routes each one in a single lookup instead of trying
 * twenty-two shapes in turn (ADR-0031).
 */
export const PlanEntryPriority = z.enum(["high", "medium", "low"])
export type PlanEntryPriority = z.infer<typeof PlanEntryPriority>

export const PlanEntryStatus = z.enum(["pending", "inProgress", "completed"])
export type PlanEntryStatus = z.infer<typeof PlanEntryStatus>

/**
 * One step of an external agent's plan.
 *
 * No identifier: in v1 of the protocol a step's identity is its position, and
 * two sends may change both its text and its state.
 */
export const PlanEntry = z.object({
  content: z.string(),
  priority: PlanEntryPriority,
  status: PlanEntryStatus,
})
export type PlanEntry = z.infer<typeof PlanEntry>

/** A mode of an external agent, or one value an option may take. `id` goes back to the agent; `name` is what the user reads. */
export const AgentChoice = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().nullable(),
})
export type AgentChoice = z.infer<typeof AgentChoice>

/**
 * What an option is about. A selector reads its own word and nothing else:
 * `other` is also what a word this build does not know becomes, never a guess
 * at a known one.
 */
export const AgentOptionCategory = z
  .enum(["mode", "model", "modelConfig", "thoughtLevel", "other"])
  .catch("other")
export type AgentOptionCategory = z.infer<typeof AgentOptionCategory>

export const AgentOptionValue = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("select"),
    current: z.string(),
    /** In the agent's order. */
    choices: z.array(AgentChoice),
  }),
  z.object({ type: z.literal("boolean"), on: z.boolean() }),
])
export type AgentOptionValue = z.infer<typeof AgentOptionValue>

export const AgentOption = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().nullable(),
  category: AgentOptionCategory,
  value: AgentOptionValue,
})
export type AgentOption = z.infer<typeof AgentOption>

/** A change asked of the conversation's agent. Identifiers are the agent's; the backend checks them against what it last declared. */
export type AgentSettingChange =
  { mode: string } | { option: string; value: string | boolean }

/**
 * What became of a settings change. `sent` carries the settings as the agent
 * answered; the panel shows those, never the value asked for.
 */
/** An agent's modes and options, whole, as it last declared them. One shape for the `agentSettings` event and for the answer to a change. */
export const AgentSettingsState = z.object({
  modes: z.array(AgentChoice),
  currentMode: z.string().nullable(),
  options: z.array(AgentOption),
})
export type AgentSettingsState = z.infer<typeof AgentSettingsState>

export const AgentSettingAnswer = z.discriminatedUnion("type", [
  /** `settings` is what the agent answered, not what was asked. */
  z.object({ type: z.literal("sent"), settings: AgentSettingsState }),
  z.object({
    type: z.literal("refused"),
    reason: z
      .enum([
        "questionInProgress",
        "unknownMode",
        "unknownOption",
        "unknownValue",
        "byAgent",
        "unknown",
      ])
      .catch("unknown"),
    /** For `byAgent`: the agent's error code, never its words. */
    code: z.number().int().nullable(),
  }),
])
export type AgentSettingAnswer = z.infer<typeof AgentSettingAnswer>

export const AiEvent = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("question"), text: z.string() }),
  z.object({
    kind: z.literal("started"),
    destination: Destination,
    tier: PrivacyTier,
    context: ContextSummary.nullable(),
    provenance: AgentProvenance.nullable(),
  }),
  z.object({ kind: z.literal("memoryReset"), reason: MemoryReset }),
  /** This conversation is not written to the workspace; the question still left. */
  z.object({ kind: z.literal("notSaved") }),
  /** Reopened: older exchanges of this conversation were not loaded. */
  z.object({ kind: z.literal("olderNotLoaded") }),
  /** Reopened: this exchange used a sample, so its answer was never written. */
  z.object({ kind: z.literal("answerNotKept") }),
  /** A tool call as the workspace kept it: no connection, no environment, no write flag. */
  z.object({
    kind: z.literal("restoredCall"),
    tool: z.string(),
    summary: z.string(),
    statement: z.string().nullable(),
    status: ToolStatus,
    errorClass: ErrorClass.nullable(),
  }),
  /** Counts only: neither a value nor a column name is kept. */
  z.object({
    kind: z.literal("sampleApproved"),
    rows: z.number().int().nonnegative(),
    columns: z.number().int().nonnegative(),
  }),
  z.object({
    kind: z.literal("turnStarted"),
    turn: z.number().int().nonnegative(),
    maxTurns: z.number().int().nonnegative(),
  }),
  z.object({ kind: z.literal("thinkingDelta"), text: z.string() }),
  z.object({ kind: z.literal("thinkingRedacted") }),
  z.object({
    kind: z.literal("thinkingEnded"),
    elapsedMs: z.number().nonnegative(),
  }),
  z.object({ kind: z.literal("textDelta"), text: z.string() }),
  z.object({
    kind: z.literal("toolDraft"),
    index: z.number().int().nonnegative(),
    tool: z.string(),
  }),
  z.object({
    kind: z.literal("toolArguments"),
    index: z.number().int().nonnegative(),
    fragment: z.string(),
  }),
  z.object({
    kind: z.literal("toolCall"),
    call: z.number().int().nonnegative(),
    tool: z.string(),
    command: z.string(),
    statement: z.string().nullable(),
    connection: z.string(),
    environment: Environment.nullable(),
    mutating: z.boolean(),
  }),
  z.object({
    kind: z.literal("approvalRequested"),
    call: z.number().int().nonnegative(),
    approval: z.string(),
    reason: z.string(),
    statement: z.string(),
    connection: z.string(),
    environment: Environment.nullable(),
    actor: freeWord<"agent">(),
  }),
  z.object({
    kind: z.literal("toolReported"),
    call: z.number().int().nonnegative(),
    status: ToolStatus,
    detail: z.string(),
    errorClass: ErrorClass.nullable(),
    withheld: z.boolean(),
    /** Rows produced or affected, measured by the executor; never parsed from `detail`. */
    rows: z.number().int().nonnegative().nullable(),
  }),
  z.object({
    kind: z.literal("callRejected"),
    tool: z.string(),
    error: z.string(),
  }),
  z.object({
    kind: z.literal("plan"),
    /** The whole plan. It replaces the previous one; it is never merged. */
    entries: z.array(PlanEntry),
  }),
  /** The whole state: it replaces the previous one; it is never merged. */
  AgentSettingsState.extend({ kind: z.literal("agentSettings") }),
  z.object({
    kind: z.literal("agentTool"),
    id: z.string(),
    tool: z.string().nullable(),
    status: AgentToolStatus,
  }),
  z.object({
    kind: z.literal("permissionRefused"),
    action: z.string(),
    reason: z.string(),
  }),
  z.object({
    kind: z.literal("usage"),
    input: z.number().int().nonnegative(),
    output: z.number().int().nonnegative(),
    cacheRead: z.number().int().nonnegative().nullable(),
    cacheWrite: z.number().int().nonnegative().nullable(),
  }),
  z.object({
    kind: z.literal("contextWindow"),
    used: z.number().nonnegative(),
    size: z.number().nonnegative(),
    cost: Money.nullable(),
  }),
  z.object({ kind: z.literal("finished"), ending: Ending }),
  z.object({
    kind: z.literal("failed"),
    message: z.string(),
    category: FailureCategory,
    /** Whether « ask again » may be offered: decided by the backend, from the error's class and the writes of the run. */
    retryable: z.boolean(),
    signIn: SignInHelp.nullable(),
    foundElsewhere: z.string().nullable(),
  }),
])
export type AiEvent = z.infer<typeof AiEvent>

export const NodeView = z.object({
  id: z.number().int().nonnegative(),
  parent: z.number().int().nonnegative().nullable(),
  question: z.string(),
  events: z.array(AiEvent),
})
export type NodeView = z.infer<typeof NodeView>

export const ThreadView = z.object({
  id: z.string(),
  title: z.string(),
  nodes: z.array(NodeView),
  selections: z.array(Selection),
  running: z.number().int().nonnegative().nullable(),
})
export type ThreadView = z.infer<typeof ThreadView>

export const AiUpdate = z.object({
  node: z.number().int().nonnegative(),
  event: AiEvent,
})
export type AiUpdate = z.infer<typeof AiUpdate>

export const SchemaProposal = z.object({
  /** Every line commented: nothing runs on a distracted `Run`. */
  sql: z.string(),
  title: z.string(),
  origin: z.string(),
})
export type SchemaProposal = z.infer<typeof SchemaProposal>

export const ProposalTarget = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("column"), name: z.string() }),
  z.object({ kind: z.literal("constraint"), name: z.string() }),
])
export type ProposalTarget = z.infer<typeof ProposalTarget>

function channel(
  stream: string,
  onUpdate: (update: AiUpdate) => void
): Channel<unknown> | null {
  // Constructing a channel outside the webview throws before `call` can say
  // why: `call` reports the missing backend instead.
  if (!isTauri()) return null
  const created = new Channel<unknown>()
  created.onmessage = guarded(stream, AiUpdate, onUpdate)
  return created
}

export const ai = {
  providers: () => call("ai_providers", z.array(DeclaredProvider)),

  externalAgents: () => call("ai_external_agents", z.array(ExternalAgent)),

  saveProvider: (draft: ProviderDraft) =>
    call("ai_save_provider", DeclaredProvider, { draft }),

  removeProvider: (id: string) => call("ai_remove_provider", Nothing, { id }),

  providerModels: (id: string) =>
    call("ai_provider_models", z.array(ModelChoice), { id }),

  /** `null` when the user declined the native confirmation. */
  saveExternalAgent: (draft: AgentDraft) =>
    call("ai_save_external_agent", ExternalAgent.nullable(), { draft }),

  removeExternalAgent: (id: string) =>
    call("ai_remove_external_agent", Nothing, { id }),

  agentPresets: () => call("ai_agent_presets", z.array(AgentPresetDraft)),

  /** Looks at the machine on the user's click. Saves nothing. */
  detectAgent: (id: AgentPresetId) =>
    call("ai_detect_agent", AgentPresetDraft, { id }),

  /** Resolves once the question is accepted; the run streams to `onUpdate`. */
  ask: (request: AskRequest, onUpdate: (update: AiUpdate) => void) =>
    call("ai_ask", AskStarted, {
      request,
      channel: channel("ai_ask", onUpdate),
    }),

  cancel: (connection: string, thread: string) =>
    call("ai_cancel", z.boolean(), { connection, thread }),

  threads: (connection: string) =>
    call("ai_threads", z.array(ThreadSummary), { connection }),

  /** A read: the conversation so far, then what follows, on `onUpdate`. */
  openThread: (
    connection: string,
    thread: string,
    onUpdate: (update: AiUpdate) => void
  ) =>
    call("ai_open_thread", ThreadView, {
      connection,
      thread,
      channel: channel("ai_open_thread", onUpdate),
    }),

  renameThread: (connection: string, thread: string, title: string) =>
    call("ai_rename_thread", Nothing, { connection, thread, title }),

  deleteThread: (connection: string, thread: string) =>
    call("ai_delete_thread", Nothing, { connection, thread }),

  selectVersion: (connection: string, thread: string, node: number) =>
    call("ai_select_version", Nothing, { connection, thread, node }),

  /** The agent signs its user in; Oxyn sees no token. */
  authenticate: (connection: string, thread: string, method: string) =>
    call("ai_authenticate", Nothing, { connection, thread, method }),

  /** Asks the agent to change a mode or an option; the panel follows the agent's declaration, not this call. */
  setAgentSetting: (
    connection: string,
    thread: string,
    change: AgentSettingChange
  ) =>
    call("ai_set_agent_setting", AgentSettingAnswer, {
      connection,
      thread,
      change,
    }),

  /**
   * Offers a sample of `address` for the next question to `destination`.
   * Refused below the `sampled` tier and for an external agent.
   */
  requestSample: (
    connection: string,
    thread: string | null,
    parent: number | null,
    address: CatalogAddress,
    destination: DestinationChoice
  ) =>
    call("ai_request_sample", SampleRequest, {
      connection,
      thread,
      parent,
      address,
      destination,
    }),

  /** The user declined the offer: its grant can no longer be presented. */
  withdrawSample: (connection: string, request: string) =>
    call("ai_withdraw_sample", Nothing, { connection, request }),

  forget: (connection: string) => call("ai_forget", Nothing, { connection }),

  proposeSchemaChange: (
    connection: string,
    address: CatalogAddress,
    target: ProposalTarget
  ) =>
    call("ai_propose_schema_change", SchemaProposal.nullable(), {
      connection,
      address,
      target,
    }),
}

export type AiBackend = typeof ai
