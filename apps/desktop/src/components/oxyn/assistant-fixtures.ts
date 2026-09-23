// Story and test data for the AI workspace. Nothing here is shown in the
// application: the panel only draws what the backend streamed.

import type {
  AssistantEntry,
  DestinationOption,
} from "@/features/assistant/availability"
import { INITIAL_ASSISTANT } from "@/features/assistant/conversation-store"
import type { AssistantState } from "@/features/assistant/conversation-store"
import { NEW_THREAD, addNode, applyUpdate } from "@/features/assistant/thread"
import type { Thread } from "@/features/assistant/thread"
import { exchangeOf } from "@/features/assistant/transcript"
import type { Exchange } from "@/features/assistant/transcript"
import type { AiEvent, DeclaredProvider, ExternalAgent } from "@/lib/ipc/ai"

/** An opaque id no story may render. */
export const APPROVAL_ID = "018f0000-0000-7000-8000-00000000a9b1"

export const remoteProvider: DeclaredProvider = {
  id: "anthropic-3f2a9b1c",
  label: "Work account",
  kind: "anthropic",
  endpoint: "https://api.anthropic.com",
  endpointRedacted: false,
  model: "claude-sonnet-5",
  keyConfigured: true,
  reach: "remote",
  measuredAtMs: Date.UTC(2026, 8, 15, 12, 30),
}

export const localProvider: DeclaredProvider = {
  id: "openai_compatible-00c0ffee",
  label: "Ollama",
  kind: "openai_compatible",
  endpoint: "http://localhost:11434/v1",
  endpointRedacted: false,
  model: "qwen2.5-coder",
  keyConfigured: false,
  reach: "local",
  measuredAtMs: Date.UTC(2026, 8, 15, 12, 30),
}

export const unresolvedProvider: DeclaredProvider = {
  id: "gemini-deadbeef",
  label: "Gemini proxy",
  kind: "gemini",
  endpoint: "https://llm-gateway.internal",
  endpointRedacted: true,
  model: "gemini-pro",
  keyConfigured: true,
  reach: "unresolved",
  measuredAtMs: 0,
}

export const externalAgent: ExternalAgent = {
  id: "agent-0badf00d",
  label: "Claude Code",
  command: "npx",
  argCount: 2,
  envNames: ["PATH"],
  preset: "claude-code",
  confined: true,
}

export const destinations: Array<DestinationOption> = [
  {
    key: `provider:${remoteProvider.id}`,
    kind: "provider",
    id: remoteProvider.id,
    label: remoteProvider.label,
    model: remoteProvider.model,
    reach: "remote",
    usable: true,
    reason: null,
  },
  {
    key: `agent:${externalAgent.id}`,
    kind: "agent",
    id: externalAgent.id,
    label: externalAgent.label,
    model: null,
    reach: "unresolved",
    usable: true,
    reason: null,
  },
]

export const enabledEntry: AssistantEntry = { status: "enabled", destinations }

/** One exchange, folded from what the backend would have streamed. */
export function exchange(question: string, events: Array<AiEvent>): Exchange {
  return exchangeOf(question, events)
}

/** A conversation of one exchange per entry, in order, the last one shown. */
export function threadOfEvents(
  exchanges: ReadonlyArray<{ question: string; events: Array<AiEvent> }>,
  { running = false }: { running?: boolean } = {}
): Thread {
  let thread = NEW_THREAD
  exchanges.forEach((entry, index) => {
    const parent = index === 0 ? null : index - 1
    thread = addNode({ ...thread, id: "42" }, index, parent, entry.question)
    for (const event of entry.events) {
      thread = applyUpdate(thread, { node: index, event })
    }
  })
  return running ? thread : { ...thread, running: null }
}

/** The panel's whole state, for a story that only draws. */
export function assistantState(
  thread: Thread,
  rest: Partial<AssistantState> = {}
): AssistantState {
  return {
    ...INITIAL_ASSISTANT,
    thread,
    history: { status: "ready", items: [] },
    ...rest,
  }
}

export const started: AiEvent = {
  kind: "started",
  destination: {
    kind: "provider",
    label: remoteProvider.label,
    model: remoteProvider.model,
    reach: "remote",
    agentVersion: null,
  },
  tier: "metadata",
  context: {
    relations: 42,
    omittedRelations: 3,
    droppedSamples: 1,
    estimatedTokens: 5_800,
  },
  provenance: {
    agent: "018f0000-0000-7000-8000-0000000a9e17",
    session: "018f0000-0000-7000-8000-00000005e551",
    kind: "anthropic",
    model: remoteProvider.model,
    at: "2026-09-15T12:31:00Z",
  },
}

export const answer = [
  "There are **1,204** active clients. The `clients` table keeps a `deleted_at`",
  "column, so the count excludes soft-deleted rows:",
  "",
  "```sql",
  "SELECT count(*)",
  "FROM public.clients",
  "WHERE deleted_at IS NULL;",
  "```",
  "",
  "- `created_at` is indexed",
  "- `status` is not: a filter on it scans the table",
].join("\n")

export const agentStarted: AiEvent = {
  kind: "started",
  destination: {
    kind: "agent",
    label: externalAgent.label,
    model: null,
    reach: "unresolved",
    agentVersion: "Claude Agent 0.78.0",
  },
  tier: "metadata",
  context: null,
  provenance: null,
}

export const answered: AiEvent = {
  kind: "finished",
  ending: { type: "answered", turns: 2, truncated: false, cut: null },
}

export const hostileAnswer = [
  "<script>window.__pwned = true</script>",
  '<img src="https://evil.example/pixel.png" onerror="window.__pwned = true">',
  "",
  "[Click to fix](javascript:window.__pwned=true) and ![tracker](https://evil.example/t.gif)",
  "",
  '<a href="https://evil.example" onclick="window.__pwned = true">raw anchor</a>',
  "",
  '<iframe src="https://evil.example"></iframe>',
].join("\n")
