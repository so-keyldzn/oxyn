// What one exchange says, independently of how it is drawn.
//
// An exchange is one question and everything its run streamed. Everything here
// is a pure function of the events the backend sent and of the decisions the
// user took: the guarantees of docs/UX-SPEC.md on this panel — a command shown
// before its result, an answer that says it was cut, a turn limit that is
// neither a success nor a failure — are facts about an array a unit test can
// assert.

import type { AssistantSource } from "@/components/oxyn/assistant-sources"
import type {
  AgentChoice,
  AgentExit,
  AgentOption,
  AgentProvenance,
  AgentToolStatus,
  AiEvent,
  CatalogReadStop,
  ContextSummary,
  Destination,
  Ending,
  ErrorClass,
  FailureCategory,
  MemoryReset,
  MentionView,
  PlanEntry,
  SampleRequest,
  SignInHelp,
  ToolStatus,
} from "@/lib/ipc/ai"
import type { CommandOutcome, Environment, PrivacyTier } from "@/lib/ipc/types"

/**
 * Where a tool call stands.
 *
 * `denied` (the policy refused: nothing can unblock it), `rejected` (the user
 * said no to an approval) and `cancelled` (the run was stopped while it ran)
 * are three different facts, and none of them is `failed`.
 */
export type ToolCallState =
  | "running"
  | "awaitingApproval"
  | "deciding"
  | "completed"
  | "denied"
  | "rejected"
  | "failed"
  | "cancelled"

export interface PendingApproval {
  /** The pending command id, passed to `decide`. Never rendered. */
  id: string
  reason: string
  statement: string
  connection: string
  environment: Environment | null
}

export interface ToolCallEntry {
  kind: "tool"
  key: string
  call: number
  tool: string
  command: string
  statement: string | null
  connection: string
  environment: Environment | null
  mutating: boolean
  state: ToolCallState
  /** The server's words, whole. */
  detail: string | null
  errorClass: ErrorClass | null
  /** The model received less than `detail`, under the connection's tier. */
  withheld: boolean
  /** Rows produced or affected, once completed. Never parsed from `detail`. */
  rows: number | null
  /**
   * The result the executor retained, to read by pages on the conversation's
   * connection. `null` when the command left none. Never rendered, never sent
   * to a model.
   */
  result: string | null
  approval: PendingApproval | null
}

/**
 * What an answer relied on: one command that ran to completion.
 *
 * `connection` is the name, never the identifier (I-03). A call that was
 * denied, rejected, cancelled or failed is not here: the answer cannot cite
 * what did not complete, and the tool call itself already says what happened.
 */
export interface Touched {
  tool: string
  command: string
  connection: string
  mutating: boolean
  /** `null` when the command measures no rows — a catalog refresh, say. */
  rows: number | null
}

export interface ThinkingEntry {
  kind: "thinking"
  key: string
  text: string
  /** The provider reasoned and shows none of it. */
  redacted: boolean
  /** `null` while the model is still reasoning. */
  elapsedMs: number | null
}

export interface ToolDraftEntry {
  kind: "toolDraft"
  key: string
  index: number
  tool: string
  /** Partial JSON, as it streams. */
  arguments: string
}

export interface FailedEntry {
  kind: "failed"
  key: string
  message: string
  category: FailureCategory
  signIn: SignInHelp | null
  foundElsewhere: string | null
  /** What the agent's process said as it died, when it did. */
  exit: AgentExit | null
  /** Asking again can change the outcome. Never after a refusal. */
  retryable: boolean
}

/**
 * Oxyn reading, from the server, the structure the assistant needs and the
 * catalog did not hold (ADR-0036). Counts only; `reading` until its account
 * arrives.
 */
export interface CatalogEntry {
  kind: "catalog"
  key: string
  reading: boolean
  listed: number
  described: number
  failed: number
  notLoaded: number
  unlisted: number
  stopped: CatalogReadStop | null
}

export type Entry =
  | { kind: "turn"; key: string; turn: number; maxTurns: number }
  | CatalogEntry
  | ThinkingEntry
  | { kind: "answer"; key: string; text: string }
  | ToolDraftEntry
  | ToolCallEntry
  | { kind: "rejectedCall"; key: string; tool: string; error: string }
  | {
      kind: "agentTool"
      key: string
      id: string
      tool: string | null
      status: AgentToolStatus
    }
  | { kind: "permissionRefused"; key: string; action: string; reason: string }
  | { kind: "memoryReset"; key: string; reason: MemoryReset }
  /** Counts only: what the sample was is not kept, here or in the backend. */
  | { kind: "sampleSent"; key: string; rows: number; columns: number }
  /** Said once per conversation: nothing of it is written to the workspace. */
  | { kind: "notSaved"; key: string }
  /** Reopened: what came before is on disk, and not loaded here. */
  | { kind: "olderNotLoaded"; key: string }
  /** Reopened: this exchange used a sample, so no answer was written. */
  | { kind: "answerNotKept"; key: string }
  | {
      kind: "restoredCall"
      key: string
      tool: string
      summary: string
      statement: string | null
      status: ToolStatus
      errorClass: ErrorClass | null
      /** It returned rows the workspace did not keep. */
      rowsNotKept: boolean
    }
  | { kind: "ended"; key: string; ending: Ending }
  | FailedEntry

export interface Started {
  destination: Destination
  tier: PrivacyTier
  context: ContextSummary | null
  provenance: AgentProvenance | null
}

export interface Usage {
  input: number
  output: number
  /** `null` while no turn declared it: unknown, not zero. */
  cacheRead: number | null
  cacheWrite: number | null
  turns: number
}

export interface AgentSettings {
  modes: ReadonlyArray<AgentChoice>
  currentMode: string | null
  options: ReadonlyArray<AgentOption>
}

export interface ContextWindow {
  used: number
  size: number
  cost: { amount: number; currency: string } | null
}

export interface Exchange {
  question: string
  /** What the question named with `@`, as the backend resolved it. */
  mentions: ReadonlyArray<MentionView>
  entries: Array<Entry>
  running: boolean
  stopRequested: boolean
  started: Started | null
  usage: Usage | null
  contextWindow: ContextWindow | null
  /**
   * The agent's plan, as last received — **never merged with the previous
   * one**. The protocol sends the whole list on every change and the client
   * replaces it, so a reducer that fuses by position or by content would
   * accumulate steps the agent has dropped.
   *
   * `null` means no plan was sent; an empty array would mean one was, and was
   * empty.
   */
  plan: ReadonlyArray<PlanEntry> | null
  /**
   * An external agent's modes and options, as it last declared them — replaced
   * whole, like the plan. `null` means none was sent: a provider, or an agent
   * that declares nothing, and then no selector is shown.
   */
  agentSettings: AgentSettings | null
  /**
   * What **Oxyn's own tools** read for this answer, derived from what went
   * through the bus (ADR-0030) — never what a model claims to have used.
   *
   * TODO(2026-11-30, unblocked by the catalog address and row count reported
   * by `ToolOutcome`, owned by ia-tauri): set by `reduce`.
   */
  sources: ReadonlyArray<AssistantSource> | null
  /**
   * The row sample an agent asked for and waits on, until the user answers —
   * drawn by the same approval screen as a pinned sample, naming the agent
   * (ADR-0034). `null` when nothing waits.
   */
  sampleAsk: SampleRequest | null
}

export function emptyExchange(
  question: string,
  mentions: ReadonlyArray<MentionView> = []
): Exchange {
  return {
    question,
    mentions,
    entries: [],
    running: false,
    stopRequested: false,
    started: null,
    usage: null,
    contextWindow: null,
    plan: null,
    agentSettings: null,
    sources: null,
    sampleAsk: null,
  }
}

function toolKey(call: number) {
  return `tool-${call}`
}

function updateTool(
  entries: Array<Entry>,
  key: string,
  change: (entry: ToolCallEntry) => ToolCallEntry
): Array<Entry> {
  if (!entries.some((entry) => entry.kind === "tool" && entry.key === key)) {
    return entries
  }
  return entries.map((entry) =>
    entry.kind === "tool" && entry.key === key ? change(entry) : entry
  )
}

function lastIndexWhere<T>(
  items: ReadonlyArray<T>,
  test: (item: T) => boolean
) {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    if (test(items[index] as T)) return index
  }
  return -1
}

function addUp(a: number | null, b: number | null) {
  return a === null && b === null ? null : (a ?? 0) + (b ?? 0)
}

/** Folds one backend event into the exchange. */
export function reduce(current: Exchange, event: AiEvent): Exchange {
  const entries = current.entries
  const at = entries.length
  const last = entries.at(-1)
  const push = (entry: Entry): Exchange => ({
    ...current,
    entries: [...entries, entry],
  })
  switch (event.kind) {
    case "question":
      return {
        ...current,
        question: event.text,
        mentions: event.mentions,
        running: true,
        stopRequested: false,
      }
    case "started":
      return {
        ...current,
        running: true,
        started: {
          destination: event.destination,
          tier: event.tier,
          context: event.context,
          provenance: event.provenance,
        },
      }
    case "memoryReset":
      return push({
        kind: "memoryReset",
        key: `m-${at}`,
        reason: event.reason,
      })
    case "catalogReading":
      return push({
        kind: "catalog",
        key: `catalog-${at}`,
        reading: true,
        listed: 0,
        described: 0,
        failed: 0,
        notLoaded: 0,
        unlisted: 0,
        stopped: null,
      })
    case "catalogRead": {
      // The account replaces the step it closes, where it was drawn.
      const index = lastIndexWhere(
        entries,
        (entry) => entry.kind === "catalog" && entry.reading
      )
      const done: CatalogEntry = {
        kind: "catalog",
        key: `catalog-${at}`,
        reading: false,
        listed: event.listed,
        described: event.described,
        failed: event.failed,
        notLoaded: event.notLoaded,
        unlisted: event.unlisted,
        stopped: event.stopped,
      }
      if (index === -1) return push(done)
      return {
        ...current,
        entries: entries.map((entry, position) =>
          position === index ? { ...done, key: entry.key } : entry
        ),
      }
    }
    case "notSaved":
      return push({ kind: "notSaved", key: `unsaved-${at}` })
    case "olderNotLoaded":
      return push({ kind: "olderNotLoaded", key: `older-${at}` })
    case "answerNotKept":
      return push({ kind: "answerNotKept", key: `notkept-${at}` })
    case "restoredCall":
      return push({
        kind: "restoredCall",
        key: `restored-${at}`,
        tool: event.tool,
        summary: event.summary,
        statement: event.statement,
        status: event.status,
        errorClass: event.errorClass,
        rowsNotKept: event.rowsNotKept,
      })
    case "sampleApproved":
      return push({
        kind: "sampleSent",
        key: `s-${at}`,
        rows: event.rows,
        columns: event.columns,
      })
    // Only while the run goes on: a reopened exchange replays the request,
    // and the call that waited for it is long gone.
    case "sampleRequested":
      return current.running
        ? { ...current, sampleAsk: event.request }
        : current
    case "sampleAnswered":
      return current.sampleAsk?.id === event.request
        ? { ...current, sampleAsk: null }
        : current
    case "turnStarted":
      return push({
        kind: "turn",
        key: `turn-${at}`,
        turn: event.turn,
        maxTurns: event.maxTurns,
      })
    case "thinkingDelta":
    case "thinkingRedacted": {
      const text = event.kind === "thinkingDelta" ? event.text : ""
      const redacted = event.kind === "thinkingRedacted"
      if (last?.kind === "thinking" && last.elapsedMs === null) {
        return {
          ...current,
          entries: [
            ...entries.slice(0, -1),
            {
              ...last,
              text: last.text + text,
              redacted: last.redacted || redacted,
            },
          ],
        }
      }
      return push({
        kind: "thinking",
        key: `th-${at}`,
        text,
        redacted,
        elapsedMs: null,
      })
    }
    case "thinkingEnded": {
      const index = lastIndexWhere(
        entries,
        (entry) => entry.kind === "thinking" && entry.elapsedMs === null
      )
      if (index < 0) return current
      return {
        ...current,
        entries: entries.map((entry, position) =>
          position === index && entry.kind === "thinking"
            ? { ...entry, elapsedMs: event.elapsedMs }
            : entry
        ),
      }
    }
    case "textDelta": {
      // An answer split around a tool call is two paragraphs, not one with a hole.
      if (last?.kind === "answer") {
        return {
          ...current,
          entries: [
            ...entries.slice(0, -1),
            { ...last, text: last.text + event.text },
          ],
        }
      }
      return push({ kind: "answer", key: `a-${at}`, text: event.text })
    }
    case "toolDraft":
      return push({
        kind: "toolDraft",
        key: `d-${at}`,
        index: event.index,
        tool: event.tool,
        arguments: "",
      })
    case "toolArguments": {
      const index = lastIndexWhere(
        entries,
        (entry) => entry.kind === "toolDraft" && entry.index === event.index
      )
      if (index < 0) return current
      return {
        ...current,
        entries: entries.map((entry, position) =>
          position === index && entry.kind === "toolDraft"
            ? { ...entry, arguments: entry.arguments + event.fragment }
            : entry
        ),
      }
    }
    case "toolCall": {
      // The oldest draft becomes this call: calls run in the order written.
      const draft = entries.findIndex((entry) => entry.kind === "toolDraft")
      const kept = draft < 0 ? entries : entries.filter((_, i) => i !== draft)
      return {
        ...current,
        entries: [
          ...kept,
          {
            kind: "tool",
            key: toolKey(event.call),
            call: event.call,
            tool: event.tool,
            command: event.command,
            statement: event.statement,
            connection: event.connection,
            environment: event.environment,
            mutating: event.mutating,
            state: "running",
            detail: null,
            errorClass: null,
            withheld: false,
            rows: null,
            result: null,
            approval: null,
          },
        ],
      }
    }
    case "approvalRequested":
      return {
        ...current,
        entries: updateTool(entries, toolKey(event.call), (entry) => ({
          ...entry,
          state: "awaitingApproval",
          approval: {
            id: event.approval,
            reason: event.reason,
            statement: event.statement,
            connection: event.connection,
            environment: event.environment,
          },
        })),
      }
    case "toolReported":
      return {
        ...current,
        entries: updateTool(entries, toolKey(event.call), (entry) =>
          // Once the user decided, the decision's outcome is the newer fact.
          entry.state !== "running" && entry.state !== "awaitingApproval"
            ? entry
            : {
                ...entry,
                // An approval the policy asked for stays open until the user
                // answers it, whatever the model did meanwhile.
                state:
                  event.status === "awaitingApproval" && entry.approval === null
                    ? "running"
                    : event.status,
                detail: event.detail,
                errorClass: event.errorClass,
                withheld: event.withheld,
                rows: event.rows,
                result: event.result,
              }
        ),
      }
    case "callRejected": {
      const draft = entries.findIndex((entry) => entry.kind === "toolDraft")
      const kept = draft < 0 ? entries : entries.filter((_, i) => i !== draft)
      return {
        ...current,
        entries: [
          ...kept,
          {
            kind: "rejectedCall",
            key: `r-${at}`,
            tool: event.tool,
            error: event.error,
          },
        ],
      }
    }
    // Replaced, never merged: the protocol sends the whole plan on every
    // change, entries have no identifier, and fusing by position would keep
    // steps the agent has dropped — growing the plan at each turn with nothing
    // failing.
    case "plan":
      return { ...current, plan: event.entries }
    // Replaced, never merged, for the same reason: the agent sends its whole
    // option list, and an option it dropped must disappear from the panel.
    case "agentSettings":
      return {
        ...current,
        agentSettings: {
          modes: event.modes,
          currentMode: event.currentMode,
          options: event.options,
        },
      }
    case "agentTool": {
      const index = entries.findIndex(
        (entry) => entry.kind === "agentTool" && entry.id === event.id
      )
      if (index >= 0) {
        return {
          ...current,
          entries: entries.map((entry, position) =>
            position === index && entry.kind === "agentTool"
              ? {
                  ...entry,
                  tool: event.tool ?? entry.tool,
                  status: event.status,
                }
              : entry
          ),
        }
      }
      return push({
        kind: "agentTool",
        key: `at-${at}`,
        id: event.id,
        tool: event.tool,
        status: event.status,
      })
    }
    case "permissionRefused":
      return push({
        kind: "permissionRefused",
        key: `p-${at}`,
        action: event.action,
        reason: event.reason,
      })
    case "usage": {
      const usage = current.usage
      return {
        ...current,
        usage: {
          input: (usage?.input ?? 0) + event.input,
          output: (usage?.output ?? 0) + event.output,
          cacheRead: addUp(usage?.cacheRead ?? null, event.cacheRead),
          cacheWrite: addUp(usage?.cacheWrite ?? null, event.cacheWrite),
          turns: (usage?.turns ?? 0) + 1,
        },
      }
    }
    case "contextWindow":
      return {
        ...current,
        contextWindow: { used: event.used, size: event.size, cost: event.cost },
      }
    case "finished": {
      const settled = settle(current)
      return {
        ...settled,
        entries: [
          ...settled.entries,
          { kind: "ended", key: `e-${at}`, ending: event.ending },
        ],
      }
    }
    case "failed": {
      const settled = settle(current)
      return {
        ...settled,
        entries: [
          ...settled.entries,
          {
            kind: "failed",
            key: `f-${at}`,
            message: event.message,
            category: event.category,
            signIn: event.signIn,
            foundElsewhere: event.foundElsewhere,
            exit: event.exit,
            // The backend's word, never deduced here from the category or the
            // message: only it knows the error's class and whether a write
            // ran before the failure (I-13, front.md).
            retryable: event.retryable,
          },
        ],
      }
    }
  }
}

/**
 * A run is over: drafts that never became calls are dropped, not left
 * spinning, and an agent's request for a sample closes with its run — the
 * backend withdrew it with the call.
 */
function settle(current: Exchange): Exchange {
  return {
    ...current,
    running: false,
    stopRequested: false,
    sampleAsk: null,
    entries: current.entries.filter((entry) => entry.kind !== "toolDraft"),
  }
}

export function exchangeOf(
  question: string,
  events: Array<AiEvent>,
  mentions: ReadonlyArray<MentionView> = []
) {
  return events.reduce(reduce, emptyExchange(question, mentions))
}

/** The user answered an approval; the backend is deciding. */
export function decisionStarted(current: Exchange, approval: string): Exchange {
  return {
    ...current,
    entries: current.entries.map((entry) =>
      entry.kind === "tool" && entry.approval?.id === approval
        ? { ...entry, state: "deciding" }
        : entry
    ),
  }
}

/** What `decide` answered for an approval, folded into its tool call. */
export function decisionSettled(
  current: Exchange,
  approval: string,
  approved: boolean,
  outcome: CommandOutcome | { error: string }
): Exchange {
  return {
    ...current,
    entries: current.entries.map((entry) => {
      if (entry.kind !== "tool" || entry.approval?.id !== approval) return entry
      if ("error" in outcome) {
        return {
          ...entry,
          state: "failed",
          detail: outcome.error,
          approval: null,
        }
      }
      if (!approved) {
        return {
          ...entry,
          state: "rejected",
          detail: "You rejected this statement. Nothing ran.",
          approval: null,
        }
      }
      switch (outcome.type) {
        case "executed":
          return {
            ...entry,
            state: outcome.cancelled ? "cancelled" : "completed",
            detail: `${outcome.rows.toLocaleString()} rows${outcome.truncated ? " (truncated)" : ""}`,
            // A cancelled run's count is partial: it is not a fact to cite.
            rows: outcome.cancelled ? null : outcome.rows,
            approval: null,
          }
        case "denied":
          return {
            ...entry,
            state: "denied",
            detail: outcome.reason,
            approval: null,
          }
        case "cancelled":
          return { ...entry, state: "cancelled", approval: null }
        default:
          return {
            ...entry,
            state: "completed",
            detail: "Done.",
            approval: null,
          }
      }
    }),
  }
}

export function stopRequested(current: Exchange): Exchange {
  return current.running ? { ...current, stopRequested: true } : current
}

/** Approvals still waiting for the user, in the order they were asked. */
export function pendingApprovals(exchange: Exchange) {
  return exchange.entries.filter(
    (entry): entry is ToolCallEntry =>
      entry.kind === "tool" &&
      entry.approval !== null &&
      (entry.state === "awaitingApproval" || entry.state === "deciding")
  )
}

/** The commands the answer relied on, in the order they ran. */
export function touched(exchange: Exchange): Touched[] {
  return exchange.entries.flatMap((entry) =>
    entry.kind === "tool" && entry.state === "completed"
      ? [
          {
            tool: entry.tool,
            command: entry.command,
            connection: entry.connection,
            mutating: entry.mutating,
            rows: entry.rows,
          },
        ]
      : []
  )
}

/** The whole answer text, as a copy puts it on the clipboard. */
export function answerText(exchange: Exchange) {
  return exchange.entries
    .filter((entry) => entry.kind === "answer")
    .map((entry) => entry.text.trim())
    .filter((text) => text !== "")
    .join("\n\n")
}

/** How the exchange ended, if it did. */
export function outcomeOf(exchange: Exchange) {
  const index = lastIndexWhere(
    exchange.entries,
    (entry) => entry.kind === "ended" || entry.kind === "failed"
  )
  const entry = exchange.entries[index]
  return entry?.kind === "ended" || entry?.kind === "failed" ? entry : undefined
}

/** Whether the exchange said anything a reader can use. */
export function saidSomething(exchange: Exchange) {
  return exchange.entries.some(
    (entry) =>
      (entry.kind === "answer" && entry.text.trim() !== "") ||
      entry.kind === "tool" ||
      entry.kind === "rejectedCall" ||
      entry.kind === "agentTool" ||
      entry.kind === "permissionRefused"
  )
}

/** Whether « Continue » makes sense after this exchange. */
export function canContinue(exchange: Exchange) {
  const outcome = outcomeOf(exchange)
  return (
    outcome?.kind === "ended" &&
    ((outcome.ending.type === "answered" && outcome.ending.truncated) ||
      outcome.ending.type === "paused" ||
      outcome.ending.type === "turnLimit" ||
      outcome.ending.type === "agentLimit")
  )
}

/** What an ending says on its last line. */
export function endingLine(ending: Ending): string {
  switch (ending.type) {
    case "answered":
      if (!ending.truncated) return `Answered in ${ending.turns} turn(s).`
      // Named from the backend's reason, never assumed: raising the token
      // limit does nothing for an answer that filled the context window.
      switch (ending.cut) {
        case "tokenLimit":
          return `Answer cut short by the token limit after ${ending.turns} turn(s). It is not finished.`
        case "contextWindow":
          return `Answer cut short: the conversation filled the model's context window after ${ending.turns} turn(s). Raising the token limit will not help; start a shorter conversation.`
        case "providerError":
          return `The provider reported an error mid-answer after ${ending.turns} turn(s). The answer is not finished.`
        default:
          return `Answer cut short after ${ending.turns} turn(s). It is not finished.`
      }
    case "cancelled":
      return ending.turns === 0
        ? "Stopped before the agent answered."
        : `Stopped after ${ending.turns} turn(s).`
    case "turnLimit":
      return `Turn limit reached after ${ending.turns} turns, with no final answer. This is neither a success nor a failure; what ran, ran.`
    case "refused":
      return "The model declined to answer this. Nothing is broken; rephrasing may help."
    case "paused":
      return "The provider paused this answer on its side. It resumes only if you continue it."
    case "agentLimit":
      return "The agent reached its own request limit for this question, with no final answer."
    case "unknown":
      return "The conversation ended in a way this Oxyn build cannot report. Nothing was hidden; nothing more is known."
  }
}

function counted(count: number, noun: string) {
  return `${count} ${noun}${count === 1 ? "" : "s"}`
}

/**
 * What the panel says of a catalog reading: the step while it runs, then what
 * it read and what the assistant was told is still missing — never a silent
 * partial schema.
 */
export function catalogLine(entry: CatalogEntry): string {
  if (entry.reading) {
    return "Reading the catalog… metadata only, no row is read."
  }
  const parts = [
    `Read the structure from the server: ${counted(entry.described, "object")} described, ${counted(entry.listed, "list")} read. No row was read.`,
  ]
  if (entry.stopped === "deadline") {
    parts.push("Stopped at the time bound.")
  } else if (entry.stopped === "cancelled") {
    parts.push("Stopped with the question.")
  }
  if (entry.failed > 0) {
    parts.push(`${counted(entry.failed, "read")} failed.`)
  }
  if (entry.notLoaded > 0 || entry.unlisted > 0) {
    parts.push(
      `Not loaded yet: ${counted(entry.notLoaded, "object")}, ${counted(entry.unlisted, "schema")} not listed. The assistant was told.`
    )
  }
  return parts.join(" ")
}

export const MEMORY_RESET_LINES: Record<MemoryReset, string> = {
  tierChanged:
    "This connection's privacy tier changed: the assistant starts over, without the earlier answers.",
  agentRestarted:
    "The agent was started again and does not remember the earlier questions of this conversation.",
  destinationChanged:
    "Someone else answers from here: the earlier answers are not part of what it knows.",
  sampleNotKept:
    "An approved sample served one question only: the assistant answers this one without the exchanges around it.",
  restarted:
    "This conversation was reopened from the workspace. What the assistant was told is not kept, so it starts over from here.",
}
