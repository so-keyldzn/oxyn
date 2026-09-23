import * as React from "react"
import {
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"

import {
  agentsQuery,
  aiKeys,
  detectedAgentQuery,
  modelsQuery,
  presetsQuery,
  providersQuery,
} from "./queries"
import { ProviderSettingsView } from "@/components/oxyn/provider-settings-view"
import type {
  ModelsState,
  ProviderSettingsFailure,
  SettingsOperation,
} from "@/components/oxyn/provider-settings-model"
import type { PresetState, PresetStates } from "@/components/oxyn/agent-presets"
import { toast } from "@/components/ui/toast"
import { ai } from "@/lib/ipc/ai"
import type {
  AgentDraft,
  AgentPresetDraft,
  AgentPresetId,
  DeclaredProvider,
  ExternalAgent,
  ProviderDraft,
} from "@/lib/ipc/ai"
import { BackendError } from "@/lib/ipc/client"

function messageOf(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

function failureOf(
  operation: SettingsOperation,
  error: unknown,
  keyMustBeRetyped: boolean
): ProviderSettingsFailure {
  return {
    operation,
    message: messageOf(error),
    // From the backend, never guessed from the message (I-13).
    retryable: error instanceof BackendError && error.retryable,
    keyMustBeRetyped,
  }
}

function presetStateOf(
  detection:
    | { data?: AgentPresetDraft; error: Error | null; isFetching: boolean }
    | undefined
): PresetState {
  if (!detection || detection.isFetching) return { status: "detecting" }
  if (detection.data) return { status: "ready", draft: detection.data }
  if (detection.error)
    return { status: "error", message: detection.error.message }
  return { status: "idle" }
}

interface AgentSave {
  draft: AgentDraft
  operation: "save-agent" | "declare-preset"
}

/**
 * The provider settings, wired to the backend.
 *
 * Every successful write invalidates the declarations: that is what makes
 * `Ask AI` appear with the first one and vanish with the last, without a
 * restart and without an event on the execution bus (docs/UX-SPEC.md).
 *
 * Nothing is shown as saved before the backend says so: the saves resolve to
 * whether they were, and the form clears only on `true`.
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
  const presetIds = (presets.data ?? []).map((preset) => preset.id)
  const detections = useQueries({
    queries: presetIds.map((id) => detectedAgentQuery(id)),
  })
  const presetStates: PresetStates = Object.fromEntries(
    presetIds.map((id, index) => [id, presetStateOf(detections[index])])
  )
  const [declaringPreset, setDeclaringPreset] =
    React.useState<AgentPresetId | null>(null)

  const settle = {
    // A new attempt replaces the failure of the previous one.
    onMutate: () => setFailure(null),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: aiKeys.all }),
  }
  const saveProvider = useMutation({
    mutationFn: (draft: ProviderDraft) => ai.saveProvider(draft),
    ...settle,
    onError: (error, draft) =>
      setFailure(failureOf("save-provider", error, draft.key !== null)),
  })
  const saveAgent = useMutation({
    mutationFn: ({ draft }: AgentSave) => ai.saveExternalAgent(draft),
    ...settle,
    onError: (error, { operation }) =>
      setFailure(failureOf(operation, error, false)),
  })
  const removeProvider = useMutation({
    mutationFn: (provider: DeclaredProvider) => ai.removeProvider(provider.id),
    ...settle,
    onError: (error) => setFailure(failureOf("remove-provider", error, false)),
  })
  const removeAgent = useMutation({
    mutationFn: (id: string) => ai.removeExternalAgent(id),
    ...settle,
    onError: (error) => setFailure(failureOf("remove-agent", error, false)),
  })

  const onSaveProvider = async (draft: ProviderDraft) => {
    try {
      await saveProvider.mutateAsync(draft)
    } catch {
      // Shown by `onError`, in the form that sent it.
      return false
    }
    toast.add({
      title: draft.id === null ? "Provider declared" : "Provider saved",
      description: draft.label,
      type: "success",
    })
    return true
  }

  /** Replacing is one write under the same id: never two agents, never none. */
  const onSaveAgent = async (draft: AgentDraft) => {
    let saved: ExternalAgent | null
    try {
      saved = await saveAgent.mutateAsync({ draft, operation: "save-agent" })
    } catch {
      return false
    }
    // Declined in the system dialog: nothing was written, nothing to say.
    if (saved === null) return false
    toast.add({
      title: draft.id === null ? "Agent declared" : "Agent replaced",
      description: draft.label,
      type: "success",
    })
    return true
  }

  /** « Detect again », for an agent installed while the screen is open. */
  const detect = (id: AgentPresetId) => {
    void queryClient.refetchQueries({ queryKey: aiKeys.detected(id) })
  }

  /** Declaring goes through the same command, and its native confirmation. */
  const declarePreset = (draft: AgentPresetDraft) => {
    setDeclaringPreset(draft.id)
    saveAgent.mutate(
      {
        draft: {
          id: null,
          label: draft.label,
          command: draft.command,
          args: draft.args,
          env: draft.env,
        },
        operation: "declare-preset",
      },
      {
        onSuccess: (saved) => {
          if (saved)
            toast.add({ title: `${draft.label} declared`, type: "success" })
        },
        onSettled: () => setDeclaringPreset(null),
      }
    )
  }

  const listModels = (provider: DeclaredProvider) => {
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
            [provider.id]: { status: "error", message: messageOf(error) },
          }))
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
      onSaveProvider={onSaveProvider}
      onSaveAgent={onSaveAgent}
      onDetectPreset={detect}
      onDeclarePreset={declarePreset}
      onRemoveProvider={(provider) => removeProvider.mutate(provider)}
      onRemoveAgent={(agent) => removeAgent.mutate(agent.id)}
      onListModels={listModels}
      onDismissFailure={() => setFailure(null)}
    />
  )
}
