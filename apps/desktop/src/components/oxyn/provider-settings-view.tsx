import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, SparklesIcon } from "@hugeicons/core-free-icons"

import { AgentPresets } from "@/components/oxyn/agent-presets"
import type { PresetStates } from "@/components/oxyn/agent-presets"
import {
  DeclaredAgentItem,
  DeclaredProviderItem,
} from "@/components/oxyn/declared-ai-list"
import { DeclarationRemovalDialog } from "@/components/oxyn/declaration-removal-dialog"
import type { Removal } from "@/components/oxyn/declaration-removal-dialog"
import { ProviderForm } from "@/components/oxyn/provider-form"
import type { FormTarget } from "@/components/oxyn/provider-form"
import {
  FAILURE_TITLES,
  isFormFailure,
} from "@/components/oxyn/provider-settings-model"
import type {
  ModelsState,
  ProviderSettingsFailure,
} from "@/components/oxyn/provider-settings-model"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import type {
  AgentDraft,
  AgentPresetDraft,
  AgentPresetId,
  DeclaredProvider,
  ExternalAgent,
  ProviderDraft,
} from "@/lib/ipc/ai"

export interface ProviderSettingsViewProps {
  status: "loading" | "ready" | "error"
  loadError?: string | null
  providers: ReadonlyArray<DeclaredProvider>
  agents: ReadonlyArray<ExternalAgent>
  /** The ready-made declarations for agents already on this machine. */
  presets: ReadonlyArray<AgentPresetDraft>
  presetStates: PresetStates
  declaringPreset: AgentPresetId | null
  working: null | "saving" | "removing"
  failure: ProviderSettingsFailure | null
  models: Readonly<Record<string, ModelsState>>
  /** Resolves `true` once saved: the form is reset only then. */
  onSaveProvider: (draft: ProviderDraft) => Promise<boolean>
  /** Resolves `true` once saved; a draft with an `id` replaces that agent. */
  onSaveAgent: (draft: AgentDraft) => Promise<boolean>
  onDetectPreset: (id: AgentPresetId) => void
  onDeclarePreset: (draft: AgentPresetDraft) => void
  onRemoveProvider: (provider: DeclaredProvider) => void
  onRemoveAgent: (agent: ExternalAgent) => void
  onListModels: (provider: DeclaredProvider) => void
  /** The failure shown no longer concerns what is on screen. */
  onDismissFailure: () => void
}

function formKey(target: FormTarget) {
  if (target?.kind === "provider") return `provider:${target.provider.id}`
  if (target?.kind === "agent") return `agent:${target.agent.id}`
  return "new"
}

/**
 * The model providers and external agents of this machine.
 *
 * Common to every workspace; the privacy tier stays on each connection
 * (ADR-0023). A key is typed once, leaves for the keychain, and is shown again
 * only as « key configured » (I-03).
 */
export function ProviderSettingsView({
  status,
  loadError = null,
  providers,
  agents,
  presets,
  presetStates,
  declaringPreset,
  working,
  failure,
  models,
  onSaveProvider,
  onSaveAgent,
  onDetectPreset,
  onDeclarePreset,
  onRemoveProvider,
  onRemoveAgent,
  onListModels,
  onDismissFailure,
}: ProviderSettingsViewProps) {
  const [target, setTarget] = React.useState<FormTarget>(null)
  const [removal, setRemoval] = React.useState<Removal | null>(null)
  const busy = working !== null || status !== "ready"
  const formFailure = isFormFailure(failure) ? failure : null
  const screenFailure = isFormFailure(failure) ? null : failure

  // A failure belongs to the form it came from: a new target starts clean.
  const retarget = (next: FormTarget) => {
    if (formFailure) onDismissFailure()
    setTarget(next)
  }

  return (
    <section
      aria-labelledby="ai-providers-title"
      className="flex flex-col gap-4"
    >
      <div className="flex flex-col gap-1">
        <h2 id="ai-providers-title" className="text-base font-medium">
          AI providers
        </h2>
        <p className="text-sm text-muted-foreground">
          Oxyn is a complete client without AI: the assistant appears only once
          a provider or an agent is declared. What each connection lets leave is
          set on the connection.
        </p>
      </div>

      {screenFailure ? (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>{FAILURE_TITLES[screenFailure.operation]}</AlertTitle>
          <AlertDescription data-selectable>
            <span className="font-mono text-xs">{screenFailure.message}</span>
            {screenFailure.retryable ? (
              <span className="block">You can try again.</span>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      {status === "loading" ? (
        <div
          role="status"
          className="flex flex-col gap-2"
          aria-busy="true"
          aria-label="Loading providers"
        >
          <Skeleton className="h-14 w-full" />
          <Skeleton className="h-14 w-full" />
        </div>
      ) : status === "error" ? (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>The declarations could not be read</AlertTitle>
          <AlertDescription data-selectable className="font-mono text-xs">
            {loadError}
          </AlertDescription>
        </Alert>
      ) : providers.length === 0 && agents.length === 0 ? (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={SparklesIcon} strokeWidth={2} />
            </EmptyMedia>
            <EmptyTitle>No provider declared</EmptyTitle>
            <EmptyDescription>
              Declare a provider below, or use an agent already on this machine.
              Ask AI then appears in the connection bar. Until then, nothing is
              sent anywhere.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="flex flex-col gap-2">
          {providers.map((provider) => (
            <DeclaredProviderItem
              key={provider.id}
              provider={provider}
              listed={models[provider.id]}
              busy={busy}
              onListModels={() => onListModels(provider)}
              onEdit={() => retarget({ kind: "provider", provider })}
              onRemove={() => setRemoval({ kind: "provider", provider })}
            />
          ))}
          {agents.map((agent) => (
            <DeclaredAgentItem
              key={agent.id}
              agent={agent}
              busy={busy}
              onReplace={() => retarget({ kind: "agent", agent })}
              onRemove={() => setRemoval({ kind: "agent", agent })}
            />
          ))}
        </div>
      )}

      <AgentPresets
        presets={presets}
        states={presetStates}
        agents={agents}
        declaring={declaringPreset}
        onDetect={onDetectPreset}
        onDeclare={onDeclarePreset}
      />

      <ProviderForm
        key={formKey(target)}
        target={target}
        working={working === "saving"}
        failure={formFailure}
        onSaveProvider={onSaveProvider}
        onSaveAgent={onSaveAgent}
        onDone={() => retarget(null)}
      />

      <DeclarationRemovalDialog
        removal={removal}
        // The declaration being edited is going: the form stops editing it.
        onRemoveProvider={(provider) => {
          if (target?.kind === "provider" && target.provider.id === provider.id)
            retarget(null)
          onRemoveProvider(provider)
        }}
        onRemoveAgent={(agent) => {
          if (target?.kind === "agent" && target.agent.id === agent.id)
            retarget(null)
          onRemoveAgent(agent)
        }}
        onClose={() => setRemoval(null)}
      />
    </section>
  )
}
