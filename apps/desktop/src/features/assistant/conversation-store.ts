// The assistant of each connection, outside React.
//
// The channel callback lives here and not in a component's effect: closing the
// panel must not stop a conversation, and a component that unmounts would take
// its listener with it. After a webview reload the module starts empty and
// `resumeAssistant` takes back what the backend kept — a read: nothing is
// asked again.
//
// Two races are closed here, not in the views:
// * an event can reach the channel before the `ask` that caused it resolves;
//   it waits in `buffered` until its node exists;
// * an event of a conversation the panel has since left belongs to an older
//   generation, and is dropped.

import * as React from "react"

import {
  NEW_THREAD,
  activePath,
  addNode,
  applyUpdate,
  dequeue,
  enqueue,
  select,
  threadOf,
  updateExchange,
} from "./thread"
import type { ExchangeNode, Thread } from "./thread"
import { refusalMessage, toAgentSettingChange } from "./agent-settings"
import { forgetAgentStart, updateStartedSettings } from "./agent-startup"
import {
  decisionSettled,
  decisionStarted,
  reduce,
  stopRequested as markStopRequested,
} from "./transcript"
import type { AgentSettingIntent } from "@/components/oxyn/assistant-agent-settings"
import { ai } from "@/lib/ipc/ai"
import type {
  AiUpdate,
  DestinationChoice,
  SampleApproval,
  SampleRequest,
  ThreadSummary,
} from "@/lib/ipc/ai"
import type { CatalogAddress } from "@/lib/ipc/types"
import { backend } from "@/lib/ipc/client"

/** What a sign-in button shows while the agent handles it. */
export type SignInState =
  | { status: "pending" }
  | { status: "done" }
  | { status: "error"; message: string }

export type HistoryState =
  | { status: "idle" }
  | { status: "loading"; items: Array<ThreadSummary> }
  | { status: "ready"; items: Array<ThreadSummary> }
  | { status: "error"; items: Array<ThreadSummary>; message: string }

export interface AssistantState {
  thread: Thread
  /** A question is being handed to the backend: a second click waits. */
  sending: boolean
  /** The backend refused the last question before it started. */
  askError: string | null
  /** Taking a conversation back after a reload, or opening one. */
  opening: boolean
  signIn: Record<string, SignInState>
  history: HistoryState
}

export const INITIAL_ASSISTANT: AssistantState = {
  thread: NEW_THREAD,
  sending: false,
  askError: null,
  opening: false,
  signIn: {},
  history: { status: "idle" },
}

/** Where a question goes, as the panel had it selected. */
export interface AskTarget {
  session: string
  destination: DestinationChoice
}

/** The text of « Continue »: sent as a follow-up, shown as such. */
export const CONTINUE_TEXT = "Continue."

interface Slot {
  state: AssistantState
  generation: number
  buffered: Array<AiUpdate>
  /** Where each queued message was meant to go. */
  queued: Map<string, AskTarget>
  listeners: Set<() => void>
}

const slots = new Map<string, Slot>()
const resumed = new Set<string>()
let queueCounter = 0

function slot(connection: string): Slot {
  let found = slots.get(connection)
  if (!found) {
    found = {
      state: INITIAL_ASSISTANT,
      generation: 0,
      buffered: [],
      queued: new Map(),
      listeners: new Set(),
    }
    slots.set(connection, found)
  }
  return found
}

function publish(
  connection: string,
  change: (state: AssistantState) => AssistantState
) {
  const found = slot(connection)
  found.state = change(found.state)
  found.listeners.forEach((notify) => notify())
}

function setThread(connection: string, change: (thread: Thread) => Thread) {
  publish(connection, (state) => ({ ...state, thread: change(state.thread) }))
}

const LAST_THREAD_KEY = "oxyn.assistant.thread."

function rememberThread(connection: string, id: string | null) {
  // A per-viewer convenience: without storage, the most recent one is opened.
  try {
    if (id === null) sessionStorage.removeItem(LAST_THREAD_KEY + connection)
    else sessionStorage.setItem(LAST_THREAD_KEY + connection, id)
  } catch {
    /* storage unavailable */
  }
}

function recalledThread(connection: string) {
  try {
    return sessionStorage.getItem(LAST_THREAD_KEY + connection)
  } catch {
    return null
  }
}

function message(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

function listener(connection: string, generation: number) {
  return (update: AiUpdate) => {
    const found = slot(connection)
    if (found.generation !== generation) return
    if (!found.state.thread.nodes.some((node) => node.id === update.node)) {
      found.buffered.push(update)
      return
    }
    const running = found.state.thread.running
    setThread(connection, (thread) => applyUpdate(thread, update))
    if (
      running === update.node &&
      (update.event.kind === "finished" || update.event.kind === "failed")
    ) {
      void refreshHistory(connection)
      // A queued message leaves after an answer — never after a failure,
      // which the user should read first — and never approves anything.
      if (update.event.kind === "finished") void drainQueue(connection)
    }
  }
}

function flushBuffered(connection: string) {
  const found = slot(connection)
  const pending = found.buffered
  found.buffered = []
  for (const update of pending) listener(connection, found.generation)(update)
}

export function subscribeAssistant(connection: string, notify: () => void) {
  const found = slot(connection)
  found.listeners.add(notify)
  return () => {
    found.listeners.delete(notify)
  }
}

export function getAssistant(connection: string): AssistantState {
  return slot(connection).state
}

export function useAssistant(connection: string): AssistantState {
  return React.useSyncExternalStore(
    React.useCallback(
      (notify: () => void) => subscribeAssistant(connection, notify),
      [connection]
    ),
    () => getAssistant(connection),
    () => getAssistant(connection)
  )
}

async function send(
  connection: string,
  target: AskTarget,
  text: string,
  parent: number | null,
  sample: SampleApproval | null = null
) {
  const found = slot(connection)
  const thread = found.state.thread
  publish(connection, (state) => ({ ...state, sending: true, askError: null }))
  const generation = found.generation
  try {
    const started = await ai.ask(
      {
        connection,
        session: target.session,
        thread: thread.id,
        parent,
        question: text,
        destination: target.destination,
        sample,
      },
      listener(connection, generation)
    )
    if (slot(connection).generation !== generation) return
    setThread(connection, (current) =>
      addNode({ ...current, id: started.thread }, started.node, parent, text)
    )
    rememberThread(connection, started.thread)
    flushBuffered(connection)
    publish(connection, (state) => ({ ...state, sending: false }))
    void refreshHistory(connection)
  } catch (error) {
    publish(connection, (state) => ({
      ...state,
      sending: false,
      askError: message(error),
    }))
    throw error
  }
}

/**
 * Asks a question after the exchange shown last.
 *
 * While a run goes on, the message is queued and sent when it ends; it is not
 * a reply to anything pending. Rejects with the backend's refusal when nothing
 * started, so the composer keeps the draft. Never retried here (I-13).
 */
export async function askQuestion(
  connection: string,
  target: AskTarget,
  text: string,
  sample: SampleApproval | null = null
): Promise<"sent" | "queued"> {
  const { state } = slot(connection)
  if (state.thread.running !== null || state.sending) {
    // The grant is bound to the exchange it follows, and a queue would send
    // it after another one: the approval would cover a question it never saw.
    if (sample !== null) throw new Error(SAMPLE_WAITS)
    const key = `queued-${(queueCounter += 1)}`
    slot(connection).queued.set(key, target)
    setThread(connection, (thread) => enqueue(thread, key, text))
    return "queued"
  }
  const last = activePath(state.thread).at(-1)
  await send(connection, target, text, last?.id ?? null, sample)
  return "sent"
}

/**
 * The user declined an offer: the backend forgets its grant instead of letting
 * it wait ten minutes. A failure leaves nothing to do — the grant still expires.
 */
export function withdrawSample(connection: string, request: SampleRequest) {
  void ai.withdrawSample(connection, request.id).catch(() => undefined)
}

export const SAMPLE_WAITS =
  "A sample goes with a question sent now. Wait for the answer, then ask for the sample again."

/**
 * Asks the backend to offer a sample for the question that would follow the
 * exchange shown last — the same place `askQuestion` sends it. Regenerating
 * or editing never reuses an approval: they send none.
 */
export function requestSample(
  connection: string,
  target: AskTarget,
  address: CatalogAddress
): Promise<SampleRequest> {
  const { thread } = slot(connection).state
  const last = activePath(thread).at(-1)
  return ai.requestSample(
    connection,
    thread.id,
    last?.id ?? null,
    address,
    target.destination
  )
}

/** Another version of an exchange: the same question, asked again. */
export function regenerate(
  connection: string,
  target: AskTarget,
  node: ExchangeNode
) {
  return send(connection, target, node.exchange.question, node.parent)
}

/** Another version with an edited question. What followed stays with the old one. */
export function editQuestion(
  connection: string,
  target: AskTarget,
  node: ExchangeNode,
  text: string
) {
  return send(connection, target, text, node.parent)
}

/** Asks the model to go on after a cut or paused answer. */
export function continueAnswer(
  connection: string,
  target: AskTarget,
  node: ExchangeNode
) {
  return send(connection, target, CONTINUE_TEXT, node.id)
}

export function selectVersion(connection: string, node: number) {
  setThread(connection, (thread) => select(thread, node))
  const id = slot(connection).state.thread.id
  if (id !== null) void ai.selectVersion(connection, id, node).catch(() => {})
}

async function drainQueue(connection: string) {
  const { state, queued } = slot(connection)
  const next = state.thread.queue[0]
  if (!next || state.thread.running !== null) return
  const target = queued.get(next.key)
  queued.delete(next.key)
  setThread(connection, (thread) => dequeue(thread, next.key))
  if (!target) return
  await askQuestion(connection, target, next.text).catch(() => undefined)
}

/** Sends a queued message now — after a failure, when the user chooses to. */
export async function sendQueued(connection: string, key: string) {
  const { state, queued } = slot(connection)
  const waiting = state.thread.queue.find((entry) => entry.key === key)
  const target = queued.get(key)
  if (!waiting || !target || state.thread.running !== null) return
  queued.delete(key)
  setThread(connection, (thread) => dequeue(thread, key))
  await askQuestion(connection, target, waiting.text)
}

export function removeQueued(connection: string, key: string) {
  slot(connection).queued.delete(key)
  setThread(connection, (thread) => dequeue(thread, key))
}

/** Taken immediately; its effect is the provider's or the agent's. */
export async function stopAssistant(connection: string) {
  const { thread } = slot(connection).state
  if (thread.running === null || thread.id === null) return
  setThread(connection, (current) =>
    updateExchange(current, thread.running ?? -1, markStopRequested)
  )
  await ai.cancel(connection, thread.id).catch(() => false)
}

/** Opens a conversation from the history, as a read. */
export async function openThread(connection: string, id: string) {
  const found = slot(connection)
  const generation = (found.generation += 1)
  found.buffered = []
  found.queued.clear()
  publish(connection, (state) => ({
    ...state,
    opening: true,
    askError: null,
    signIn: {},
  }))
  try {
    const view = await ai.openThread(
      connection,
      id,
      listener(connection, generation)
    )
    if (slot(connection).generation !== generation) return
    publish(connection, (state) => ({
      ...state,
      thread: threadOf(view),
      opening: false,
    }))
    rememberThread(connection, id)
    flushBuffered(connection)
  } catch (error) {
    if (slot(connection).generation !== generation) return
    rememberThread(connection, null)
    publish(connection, (state) => ({
      ...state,
      thread: NEW_THREAD,
      opening: false,
      askError: message(error),
    }))
  }
}

/** A blank conversation. The one left keeps running and stays in the history. */
export function newThread(connection: string) {
  const found = slot(connection)
  found.generation += 1
  found.buffered = []
  found.queued.clear()
  rememberThread(connection, null)
  publish(connection, (state) => ({
    ...state,
    thread: NEW_THREAD,
    askError: null,
    signIn: {},
  }))
}

/** Takes back the conversation the backend kept, once per webview load. */
export async function resumeAssistant(connection: string) {
  if (resumed.has(connection)) return
  resumed.add(connection)
  const items = await refreshHistory(connection)
  if (items === null) {
    resumed.delete(connection)
    return
  }
  const recalled = recalledThread(connection)
  const chosen =
    items.find((item) => item.id === recalled) ??
    items.find((item) => item.running)
  if (chosen) await openThread(connection, chosen.id)
}

export async function refreshHistory(
  connection: string
): Promise<Array<ThreadSummary> | null> {
  const previous = slot(connection).state.history
  const items = "items" in previous ? previous.items : []
  publish(connection, (state) => ({
    ...state,
    history: { status: "loading", items },
  }))
  try {
    const listed = await ai.threads(connection)
    publish(connection, (state) => ({
      ...state,
      history: { status: "ready", items: listed },
    }))
    return listed
  } catch (error) {
    publish(connection, (state) => ({
      ...state,
      history: { status: "error", items, message: message(error) },
    }))
    return null
  }
}

export async function renameThread(
  connection: string,
  id: string,
  title: string
) {
  await ai.renameThread(connection, id, title)
  if (slot(connection).state.thread.id === id) {
    setThread(connection, (thread) => ({ ...thread, title: title.trim() }))
  }
  await refreshHistory(connection)
}

export async function deleteThread(connection: string, id: string) {
  await ai.deleteThread(connection, id)
  if (slot(connection).state.thread.id === id) newThread(connection)
  await refreshHistory(connection)
}

/** Lets the agent run one of its own sign-in methods; Oxyn sees no token. */
export async function signIn(connection: string, method: string) {
  const id = slot(connection).state.thread.id
  if (id === null) return
  const mark = (value: SignInState) =>
    publish(connection, (state) => ({
      ...state,
      signIn: { ...state.signIn, [method]: value },
    }))
  mark({ status: "pending" })
  try {
    await ai.authenticate(connection, id, method)
    mark({ status: "done" })
  } catch (error) {
    mark({ status: "error", message: message(error) })
  }
}

/**
 * Answers an agent's approval through the existing `decide`: the policy
 * decides again, with the agent as actor (I-02, I-07). Only this click
 * approves — a message typed meanwhile approves nothing.
 */
export async function decideApproval(
  connection: string,
  node: number,
  approval: string,
  approved: boolean
) {
  setThread(connection, (thread) =>
    updateExchange(thread, node, (exchange) =>
      decisionStarted(exchange, approval)
    )
  )
  const settled = await backend.decide(approval, approved).then(
    (outcome) => outcome,
    (error: unknown) => ({ error: message(error) })
  )
  setThread(connection, (thread) =>
    updateExchange(thread, node, (exchange) =>
      decisionSettled(exchange, approval, approved, settled)
    )
  )
}

/**
 * Asks the conversation's agent to change one of its settings.
 *
 * A `sent` reply carries the settings **as the agent answered them**, and they
 * replace the shown ones at once: between two questions no `agentSettings`
 * event may ever come. They go through the same `reduce` as that event, onto
 * the latest exchange, so an event arriving later replaces them in turn —
 * whichever arrived last is what the panel shows, with no second copy to keep
 * in step.
 *
 * A refusal changes nothing and rejects with a sentence: the selector shows it
 * after the setting's name. A conversation left in the meantime is not written
 * into — those settings were not its own.
 */
export async function changeAgentSetting(
  connection: string,
  intent: AgentSettingIntent
) {
  // Before the first question there is no conversation: the backend then
  // asks the agent the panel started, which is also the one it asks first
  // when there is one — it answers the next question.
  const thread = getAssistant(connection).thread.id
  const answer = await ai.setAgentSetting(
    connection,
    thread,
    toAgentSettingChange(intent)
  )
  if (answer.type === "refused") throw new Error(refusalMessage(answer))
  updateStartedSettings(connection, answer.settings)
  if (thread === null) return
  setThread(connection, (current) => {
    const latest = activePath(current).at(-1)
    if (current.id !== thread || !latest) return current
    return updateExchange(current, latest.id, (exchange) =>
      reduce(exchange, { kind: "agentSettings", ...answer.settings })
    )
  })
}

/** Drops a connection's conversations; call when the connection closes. */
export async function closeConversation(connection: string) {
  const found = slot(connection)
  found.generation += 1
  found.buffered = []
  found.queued.clear()
  resumed.delete(connection)
  rememberThread(connection, null)
  publish(connection, () => INITIAL_ASSISTANT)
  // The backend releases the agent started for it with the rest.
  forgetAgentStart(connection)
  await ai.forget(connection).catch(() => undefined)
}
