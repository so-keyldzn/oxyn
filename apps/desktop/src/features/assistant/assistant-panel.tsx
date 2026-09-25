import * as React from "react"
import { useQuery } from "@tanstack/react-query"

import {
  answerSampleAsk,
  askQuestion,
  changeAgentSetting,
  continueAnswer,
  decideApproval,
  deleteThread,
  editQuestion,
  newThread,
  openThread,
  refreshHistory,
  regenerate,
  removeQueued,
  renameThread,
  requestSample,
  SAMPLE_WAITS,
  withdrawSample,
  resumeAssistant,
  selectVersion,
  sendQueued,
  signIn,
  stopAssistant,
  useAssistant,
} from "./conversation-store"
import type { AskTarget } from "./conversation-store"
import {
  signInStartedAgent,
  startAgent,
  startAgentAgain,
  stopAgentStart,
  useAgentStartup,
} from "./agent-startup"
import { activePath } from "./thread"
import { destinationChoice, selectedDestination } from "./availability"
import { chooseDestination, unpin, usePinState } from "./object-pin"
import type { ObjectPin } from "./object-pin"
import { useProviderModels } from "./use-provider-models"
import { declaredEfforts, effortToSend } from "./reasoning-effort"
import type { ExchangeNode } from "./thread"
import { useAssistantAvailable } from "./use-assistant-available"
import { useMentionSearch } from "./mention-search"
import { orphanThreadsQuery, prunedHistoryQuery } from "./queries"
import { ToolRows } from "./tool-rows"
import { ErdBlock } from "./erd-block"
import { AssistantEntryButton } from "@/components/oxyn/assistant-entry-button"
import type { AgentStartupControls } from "@/components/oxyn/assistant-agent-startup"
import type { ModelListState } from "@/components/oxyn/assistant-header"
import { AssistantSampleApproval } from "@/components/oxyn/assistant-sample-approval"
import { AssistantView } from "@/components/oxyn/assistant-view"
import { toast } from "@/components/ui/toast"
import { openObject } from "@/features/workspace/object-requests"
import type {
  AgentProvenance,
  Mention,
  ReasoningEffort,
  SampleRequest,
} from "@/lib/ipc/ai"
import type { OpenConnection } from "@/lib/ipc/types"

/** Copies, and says so only when it fails: a tick already says it worked. */
async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch (error) {
    toast.add({
      title: "Not copied",
      description: error instanceof Error ? error.message : String(error),
      type: "error",
    })
    return false
  }
}

/**
 * The assistant of an open connection, wired to the backend.
 *
 * Draws nothing when no provider or agent is declared. Closing it does not
 * stop a conversation; closing the connection should
 * (`closeConversation`, docs/UX-SPEC.md).
 */
export function AssistantPanel({
  open,
  onOpenInConsole,
}: {
  open: OpenConnection
  /** Drops the text in a console, with the provenance that signs it (ADR-0023). */
  onOpenInConsole: (sql: string, provenance: AgentProvenance | null) => void
}) {
  const entry = useAssistantAvailable(open)
  const state = useAssistant(open.connection)
  const orphanThreads = useQuery(orphanThreadsQuery)
  const prunedHistory = useQuery(prunedHistoryQuery)
  // From the local catalog and library: no model completes a name.
  const mentionSource = useMentionSearch(open.connection)
  const { chosenKey, pin: pinned } = usePinState(open.connection)
  const [chosenModels, setChosenModels] = React.useState<
    Record<string, string>
  >({})
  // Kept beside the model, by provider: the same place, not a second store.
  const [chosenEfforts, setChosenEfforts] = React.useState<
    Record<string, ReasoningEffort>
  >({})

  React.useEffect(() => {
    void resumeAssistant(open.connection)
  }, [open.connection])

  const destinations = entry.status === "absent" ? [] : entry.destinations
  const selected = selectedDestination(destinations, chosenKey)
  // The pin stands only where a sample can follow: the menu offered it under
  // the same condition, and either side may have changed since. A provider or
  // an external agent: the sample enters either prompt by the same gate.
  const pinnable =
    entry.status === "enabled" &&
    open.privacyTier === "sampled" &&
    selected !== null &&
    selected.usable
  const pin = pinnable ? pinned : null
  React.useEffect(() => {
    if (!pinnable && pinned !== null) unpin(open.connection)
  }, [pinnable, pinned, open.connection])
  const sampleApproval = useSampleApproval()
  const agentAsk = useAgentSampleAsk(
    open.connection,
    state.thread.nodes.find((node) => node.id === state.thread.running) ?? null
  )
  const model =
    selected?.kind === "provider"
      ? (chosenModels[selected.id] ?? selected.model)
      : null
  const models = useProviderModels(selected)
  // `isPending` alone is also true for a query never enabled: an agent, or a
  // provider the tier refuses, would read as « listing ».
  const modelList: ModelListState | null = models.isError
    ? { status: "error", message: models.error.message }
    : models.isPending && models.fetchStatus !== "idle"
      ? { status: "loading" }
      : null

  const startup = useAgentStartup(open.connection)
  const running = state.thread.running !== null || state.sending
  const parent = activePath(state.thread).at(-1)?.id ?? null
  const agentToStart =
    entry.status === "enabled" && selected?.kind === "agent" && selected.usable
      ? { id: selected.id, label: selected.label }
      : null
  const agentId = agentToStart?.id ?? null
  const agentLabel = agentToStart?.label ?? ""
  // The agent the next question would use, started as soon as it is shown, so
  // its models and options are offered before anything is asked. Not while a
  // question runs — that one starts it — nor while a conversation is being
  // taken back: its own agent may be the one that answers next. A start
  // already made for the same agent, session and question is not made again,
  // whatever its end (`startAgent`).
  React.useEffect(() => {
    if (agentId === null) {
      void stopAgentStart(open.connection)
      return
    }
    if (running || state.opening) return
    void startAgent(
      open.connection,
      {
        connection: open.connection,
        session: open.session,
        thread: state.thread.id,
        parent,
        agent: agentId,
      },
      agentLabel
    )
  }, [
    open.connection,
    open.session,
    agentId,
    agentLabel,
    state.thread.id,
    parent,
    running,
    state.opening,
  ])
  const agentStartup: AgentStartupControls | null =
    agentId === null
      ? null
      : {
          startup: startup.state,
          signInStates: startup.signIn,
          onCancel: () => void stopAgentStart(open.connection, true),
          onStart: () => startAgentAgain(open.connection),
          onSignIn: (method) =>
            void signInStartedAgent(open.connection, method),
          onCopy: copy,
        }

  const target: AskTarget | null = selected
    ? {
        session: open.session,
        destination: destinationChoice(
          selected,
          model !== selected.model ? model : null,
          selected.kind === "provider"
            ? effortToSend(
                chosenEfforts[selected.id] ?? null,
                models.data ?? null,
                model
              )
            : null
        ),
      }
    : null

  /** Every send goes through here: without a destination, nothing leaves. */
  const withTarget = async (
    run: (target: AskTarget) => Promise<unknown>
  ): Promise<boolean> => {
    if (!target) return false
    try {
      await run(target)
      return true
    } catch {
      // The store already published the refusal; the composer keeps the draft.
      return false
    }
  }

  /** The pinned object's sample, approved column by column, then the question. */
  const askWithSample = async (
    asked: AskTarget,
    question: string,
    object: ObjectPin,
    mentions: Array<Mention>
  ) => {
    if (state.thread.running !== null || state.sending) {
      toast.add({ title: "Not sent", description: SAMPLE_WAITS, type: "error" })
      return false
    }
    let request: SampleRequest
    try {
      request = await requestSample(open.connection, asked, object.address)
    } catch (error) {
      toast.add({
        title: "No sample offered",
        description: error instanceof Error ? error.message : String(error),
        type: "error",
      })
      return false
    }
    const columns = await sampleApproval.ask(request)
    if (columns === null || columns.length === 0) {
      withdrawSample(open.connection, request)
      return false
    }
    try {
      await askQuestion(
        open.connection,
        asked,
        question,
        {
          request: request.id,
          source: request.address,
          columns: [...columns],
        },
        mentions
      )
    } finally {
      sampleApproval.settle()
    }
    // One question: the next one asks again (docs/AI-PROVIDERS.md).
    unpin(open.connection)
    return true
  }

  return (
    <>
      <AssistantView
        connectionName={open.name}
        environment={open.environment}
        tier={open.privacyTier}
        entry={entry}
        state={state}
        selected={selected}
        model={model}
        models={models.data ?? null}
        modelCost={
          models.data?.find((choice) => choice.id === model)?.cost ?? null
        }
        onSelectDestination={(key) => chooseDestination(open.connection, key)}
        onSelectModel={(chosen) => {
          if (selected?.kind === "provider")
            setChosenModels((all) => ({ ...all, [selected.id]: chosen }))
        }}
        efforts={declaredEfforts(models.data ?? null, model)}
        effort={
          selected?.kind === "provider"
            ? (chosenEfforts[selected.id] ?? null)
            : null
        }
        onSelectEffort={(chosen) => {
          if (selected?.kind === "provider")
            setChosenEfforts((all) => ({ ...all, [selected.id]: chosen }))
        }}
        agentStartup={agentStartup}
        modelList={modelList}
        pins={pin ? [{ key: pin.key, kind: "object", label: pin.label }] : []}
        onRemovePin={() => unpin(open.connection)}
        mentionSource={mentionSource}
        onAsk={(question, mentions) =>
          pin && target
            ? // A refusal is already published by the store; the draft stays.
              askWithSample(target, question, pin, mentions).catch(() => false)
            : withTarget((asked) =>
                askQuestion(open.connection, asked, question, null, mentions)
              )
        }
        onStop={() => void stopAssistant(open.connection)}
        onDecide={(node, approval, approved) =>
          void decideApproval(open.connection, node, approval, approved)
        }
        onOpenInConsole={onOpenInConsole}
        // In this connection's workspace, as a click in its catalog would.
        onOpenObject={(address) => openObject(open.connection, address)}
        // The rows are read from this connection's executor, for the user.
        renderToolRows={(call) =>
          call.result === null ? null : (
            <ToolRows
              connection={open.connection}
              entry={{ ...call, result: call.result }}
            />
          )
        }
        // Names resolved against this connection's catalog, never believed.
        renderErd={(request) => (
          <ErdBlock
            open={open}
            request={request}
            onOpenObject={(address) => openObject(open.connection, address)}
          />
        )}
        onChangeAgentSetting={(intent) =>
          changeAgentSetting(open.connection, intent)
        }
        onRegenerate={(node: ExchangeNode) =>
          void withTarget((asked) => regenerate(open.connection, asked, node))
        }
        onEdit={(node, text) =>
          withTarget((asked) =>
            editQuestion(open.connection, asked, node, text)
          )
        }
        onContinue={(node) =>
          void withTarget((asked) =>
            continueAnswer(open.connection, asked, node)
          )
        }
        onSelectVersion={(node) => selectVersion(open.connection, node)}
        onSignIn={(method) => void signIn(open.connection, method)}
        onCopy={copy}
        onNewConversation={() => newThread(open.connection)}
        onOpenThread={(id) => void openThread(open.connection, id)}
        onRenameThread={(id, title) => renameThread(open.connection, id, title)}
        onDeleteThread={(id) => deleteThread(open.connection, id)}
        onReloadHistory={() => {
          void refreshHistory(open.connection)
          void orphanThreads.refetch()
        }}
        // A connection deleted in settings says nothing to this panel.
        onHistoryShown={() => {
          void orphanThreads.refetch()
          void prunedHistory.refetch()
        }}
        pruned={prunedHistory.data ?? null}
        orphans={
          orphanThreads.isError
            ? { status: "error", message: orphanThreads.error.message }
            : orphanThreads.data
              ? { status: "ready", items: orphanThreads.data }
              : undefined
        }
        onSendQueued={(key) => void sendQueued(open.connection, key)}
        onRemoveQueued={(key) => removeQueued(open.connection, key)}
      />
      {/* The user's own pin first: it is what they are doing now. An agent's
            request waits behind it, and its call waits with it. */}
      <AssistantSampleApproval
        request={sampleApproval.request ?? agentAsk.request}
        connectionName={open.name}
        environment={open.environment}
        tier={open.privacyTier}
        deciding={
          sampleApproval.request !== null
            ? sampleApproval.deciding
            : agentAsk.deciding
        }
        onDecide={
          sampleApproval.request !== null
            ? sampleApproval.decide
            : agentAsk.decide
        }
      />
    </>
  )
}

/**
 * The request for a sample an agent waits on, in the running exchange — and
 * the user's answer to it, sent once.
 *
 * Answered here only: nothing the agent sends can approve it (ADR-0034). The
 * screen closes when the backend says the request was answered, or when the
 * run ends; until then it stays open and inert, so a second click cannot
 * answer twice.
 */
function useAgentSampleAsk(connection: string, running: ExchangeNode | null) {
  const request = running?.exchange.sampleAsk ?? null
  const [answered, setAnswered] = React.useState<string | null>(null)
  const deciding = request !== null && answered === request.id
  return {
    request,
    deciding,
    decide: (columns: ReadonlyArray<string> | null) => {
      if (request === null || deciding) return
      setAnswered(request.id)
      answerSampleAsk(
        connection,
        request.id,
        columns === null || columns.length === 0 ? null : columns
      ).catch((error: unknown) => {
        // Expired or withdrawn meanwhile: the backend already told the agent
        // « declined », and the screen closes with the run.
        toast.add({
          title: "Sample not sent",
          description: error instanceof Error ? error.message : String(error),
          type: "error",
        })
      })
    },
  }
}

/**
 * One approval at a time, awaited by the send that asked for it.
 *
 * The screen closes on the decision; while the question is being accepted it
 * stays open and inert, so a second click cannot approve twice.
 */
function useSampleApproval() {
  const [request, setRequest] = React.useState<SampleRequest | null>(null)
  const [deciding, setDeciding] = React.useState(false)
  const answer = React.useRef<
    ((columns: ReadonlyArray<string> | null) => void) | null
  >(null)
  return {
    request,
    deciding,
    ask: (offered: SampleRequest) =>
      new Promise<ReadonlyArray<string> | null>((resolve) => {
        answer.current?.(null)
        answer.current = resolve
        setDeciding(false)
        setRequest(offered)
      }),
    decide: (columns: ReadonlyArray<string> | null) => {
      const resolve = answer.current
      answer.current = null
      if (columns === null || columns.length === 0) setRequest(null)
      else setDeciding(true)
      resolve?.(columns)
    },
    settle: () => {
      setDeciding(false)
      setRequest(null)
    },
  }
}

/** `Ask AI` for the connection bar: nothing at all without a declaration. */
export function AskAiButton({
  open,
  pressed,
  onPressedChange,
}: {
  open: OpenConnection
  pressed: boolean
  onPressedChange: (pressed: boolean) => void
}) {
  const entry = useAssistantAvailable(open)
  return (
    <AssistantEntryButton
      entry={entry}
      pressed={pressed}
      onPressedChange={onPressedChange}
    />
  )
}
