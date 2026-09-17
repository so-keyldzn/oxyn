import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import {
  agentsQuery,
  aiKeys,
  modelsQuery,
  presetsQuery,
  providersQuery,
} from "./queries"
import { ProviderSettingsView } from "@/components/oxyn/provider-settings-view"
import type {
  ModelsState,
  ProviderSettingsFailure,
} from "@/components/oxyn/provider-settings-view"
import type { PresetStates } from "@/components/oxyn/agent-presets"
import { ai } from "@/lib/ipc/ai"
import type {
  AgentDraft,
  AgentPresetDraft,
  AgentPresetId,
  DeclaredProvider,
  ProviderDraft,
} from "@/lib/ipc/ai"
import { BackendError } from "@/lib/ipc/client"

function failureOf(
  error: unknown,
  keyMustBeRetyped: boolean
): ProviderSettingsFailure {
  return {
    message: error instanceof Error ? error.message : String(error),
    // From the backend, never guessed from the message (I-13).
    retryable: error instanceof BackendError && error.retryable,
    keyMustBeRetyped,
  }
}

/**
 * The provider settings, wired to the backend.
 *
 * Every successful write invalidates the declarations: that is what makes
 * `Ask AI` appear with the first one and vanish with the last, without a
 * restart and without an event on the execution bus (docs/UX-SPEC.md).
 */
export function ProviderSettings() {
  const queryClient = useQueryClient()
  const providers = useQuery(providersQuery)
  const agents = useQuery(agentsQuery)
  const [failure, setFailure] = React.useState<ProviderSettingsFailure | null>(
    null
  )
  const [models, setModels] = React.useState<Record<string, ModelsState>>({})
  const presets = useQuery(presetsQuery)
  const [presetStates, setPresetStates] = React.useState<PresetStates>({})
  const [declaringPreset, setDeclaringPreset] =
    React.useState<AgentPresetId | null>(null)

  const settle = {
    onSuccess: () => {
      setFailure(null)
      return queryClient.invalidateQueries({ queryKey: aiKeys.all })
    },
  }
  const saveProvider = useMutation({
    mutationFn: (draft: ProviderDraft) => ai.saveProvider(draft),
    ...settle,
    onError: (error, draft) => setFailure(failureOf(error, draft.key !== null)),
  })
  const saveAgent = useMutation({
    mutationFn: (draft: AgentDraft) => ai.saveExternalAgent(draft),
    ...settle,
    onError: (error) => setFailure(failureOf(error, false)),
  })
  const removeProvider = useMutation({
    mutationFn: (provider: DeclaredProvider) => ai.removeProvider(provider.id),
    ...settle,
    onError: (error) => setFailure(failureOf(error, false)),
  })
  const removeAgent = useMutation({
    mutationFn: (id: string) => ai.removeExternalAgent(id),
    ...settle,
    onError: (error) => setFailure(failureOf(error, false)),
  })

  /** On the user's click only: it reads the machine, and saves nothing. */
  const detect = (id: AgentPresetId) => {
    setPresetStates((all) => ({ ...all, [id]: { status: "detecting" } }))
    void ai.detectAgent(id).then(
      (draft) =>
        setPresetStates((all) => ({
          ...all,
          [id]: { status: "ready", draft },
        })),
      (error: unknown) =>
        setPresetStates((all) => ({
          ...all,
          [id]: {
            status: "error",
            message: error instanceof Error ? error.message : String(error),
          },
        }))
    )
  }

  /** Declaring goes through the same command, and its native confirmation. */
  const declarePreset = (draft: AgentPresetDraft) => {
    setDeclaringPreset(draft.id)
    saveAgent.mutate(
      {
        label: draft.label,
        command: draft.command,
        args: draft.args,
        env: draft.env,
      },
      { onSettled: () => setDeclaringPreset(null) }
    )
  }

  const working =
    saveProvider.isPending || saveAgent.isPending
      ? "saving"
      : removeProvider.isPending || removeAgent.isPending
        ? "removing"
        : null

  return (
    <ProviderSettingsView
      status={
        providers.isError || agents.isError
          ? "error"
          : providers.data && agents.data
            ? "ready"
            : "loading"
      }
      loadError={providers.error?.message ?? agents.error?.message ?? null}
      providers={providers.data ?? []}
      agents={agents.data ?? []}
      presets={presets.data ?? []}
      presetStates={presetStates}
      declaringPreset={declaringPreset}
      working={working}
      failure={failure}
      models={models}
      onSaveProvider={(draft) => saveProvider.mutate(draft)}
      onSaveAgent={(draft) => saveAgent.mutate(draft)}
      onDetectPreset={detect}
      onDeclarePreset={declarePreset}
      onRemoveProvider={(provider) => removeProvider.mutate(provider)}
      onRemoveAgent={(agent) => removeAgent.mutate(agent.id)}
      onCheckModels={(provider) => {
        setModels((all) => ({ ...all, [provider.id]: { status: "loading" } }))
        // A fresh request each time: it doubles as « does the endpoint answer ».
        void queryClient
          .fetchQuery({ ...modelsQuery(provider.id), staleTime: 0 })
          .then(
            (listed) =>
              setModels((all) => ({
                ...all,
                [provider.id]: { status: "ready", models: listed },
              })),
            (error: unknown) =>
              setModels((all) => ({
                ...all,
                [provider.id]: {
                  status: "error",
                  message:
                    error instanceof Error ? error.message : String(error),
                },
              }))
          )
      }}
    />
  )
}
