import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, ArrowLeft01Icon } from "@hugeicons/core-free-icons"

import { AgentPresets } from "@/components/oxyn/agent-presets"
import type { PresetStates } from "@/components/oxyn/agent-presets"
import {
  DeclaredAgentItem,
  DeclaredProviderItem,
} from "@/components/oxyn/declared-ai-list"
import { DeclarationRemovalDialog } from "@/components/oxyn/declaration-removal-dialog"
import type { Removal } from "@/components/oxyn/declaration-removal-dialog"
import { ProviderForm } from "@/components/oxyn/provider-form"
import type { FormKind, FormTarget } from "@/components/oxyn/provider-form"
import {
  ProviderList,
  ProviderLogoTile,
  ProviderStatusPill,
} from "@/components/oxyn/provider-list"
import { providerRows } from "@/components/oxyn/provider-list-model"
import type { ProviderRow } from "@/components/oxyn/provider-list-model"
import {
  FAILURE_TITLES,
  isFormFailure,
} from "@/components/oxyn/provider-settings-model"
import type {
  ModelsState,
  ProviderSettingsFailure,
} from "@/components/oxyn/provider-settings-model"
import type { ListDraftModels } from "@/components/oxyn/use-draft-models"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
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
  /** The models of the provider being typed in the form; saves nothing. */
  onListDraftModels: ListDraftModels
  /** The failure shown no longer concerns what is on screen. */
  onDismissFailure: () => void
}

function formKey(target: FormTarget) {
  if (target?.kind === "provider") return `provider:${target.provider.id}`
  if (target?.kind === "agent") return `agent:${target.agent.id}`
  return "new"
}

/** The declarations that live under a row, and the kind its form starts on. */
function contentsOf(
  row: ProviderRow,
  providers: ReadonlyArray<DeclaredProvider>,
  agents: ReadonlyArray<ExternalAgent>
): {
  providers: ReadonlyArray<DeclaredProvider>
  agents: ReadonlyArray<ExternalAgent>
  kind: FormKind
} {
  const { subject } = row
  if (subject.kind === "family")
    return {
      providers: providers.filter((p) => p.kind === subject.family),
      agents: [],
      kind: subject.family,
    }
  if (subject.kind === "preset")
    return {
      providers: [],
      agents: agents.filter((a) => a.preset === subject.preset.id),
      kind: "agent",
    }
  return {
    providers: [],
    agents: agents.filter((a) => a.preset === null),
    kind: "agent",
  }
}

/**
 * The model providers and external agents of this machine.
 *
 * A list of every destination Oxyn can declare, each opening on what it holds:
 * its declarations, their models, the form that declares or edits one. Common
 * to every workspace; the privacy tier stays on each connection (ADR-0023). A
 * key is typed once, leaves for the keychain, and is shown again only as
 * « key configured » (I-03).
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
  onListDraftModels,
  onDismissFailure,
}: ProviderSettingsViewProps) {
  const [openRow, setOpenRow] = React.useState<string | null>(null)
  // The row to focus again once back on the list.
  const [cameFrom, setCameFrom] = React.useState<string | null>(null)
  const [target, setTarget] = React.useState<FormTarget>(null)
  const [removal, setRemoval] = React.useState<Removal | null>(null)
  const backRef = React.useRef<HTMLButtonElement>(null)
  const busy = working !== null || status !== "ready"
  const formFailure = isFormFailure(failure) ? failure : null
  const screenFailure = isFormFailure(failure) ? null : failure

  const rows = providerRows({
    providers,
    agents,
    presets,
    presetStates,
    models,
  })
  // A row that vanished — its last custom declaration removed elsewhere —
  // leaves the list shown rather than an empty page.
  const row =
    status === "ready" ? rows.find((r) => r.id === openRow) : undefined
  const held = row ? contentsOf(row, providers, agents) : null

  // A failure belongs to the form it came from: a new target starts clean.
  const retarget = (next: FormTarget) => {
    if (formFailure) onDismissFailure()
    setTarget(next)
  }

  const open = (next: ProviderRow) => {
    retarget(null)
    setOpenRow(next.id)
  }

  const back = () => {
    retarget(null)
    setCameFrom(openRow)
    setOpenRow(null)
  }

  React.useEffect(() => {
    if (openRow !== null) backRef.current?.focus()
  }, [openRow])

  const failureAlert = screenFailure ? (
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
  ) : null

  return (
    <section
      aria-labelledby="ai-providers-title"
      className="flex flex-col gap-4"
    >
      {row === undefined || held === null ? (
        <>
          {failureAlert}
          <ProviderList
            status={status}
            loadError={loadError}
            rows={rows}
            focusRow={cameFrom}
            onOpen={open}
          />
        </>
      ) : (
        <ProviderDetail
          row={row}
          backRef={backRef}
          onBack={back}
          failureAlert={failureAlert}
        >
          {held.providers.length > 0 || held.agents.length > 0 ? (
            <ul aria-label="Declared" className="flex flex-col gap-2">
              {held.providers.map((provider) => (
                <li key={provider.id}>
                  <DeclaredProviderItem
                    provider={provider}
                    listed={models[provider.id]}
                    busy={busy}
                    onListModels={() => onListModels(provider)}
                    onEdit={() => retarget({ kind: "provider", provider })}
                    onRemove={() => setRemoval({ kind: "provider", provider })}
                  />
                </li>
              ))}
              {held.agents.map((agent) => (
                <li key={agent.id}>
                  <DeclaredAgentItem
                    agent={agent}
                    busy={busy}
                    onReplace={() => retarget({ kind: "agent", agent })}
                    onRemove={() => setRemoval({ kind: "agent", agent })}
                  />
                </li>
              ))}
            </ul>
          ) : null}

          {row.subject.kind === "preset" ? (
            <AgentPresets
              presets={[row.subject.preset]}
              states={presetStates}
              agents={agents}
              declaring={declaringPreset}
              onDetect={onDetectPreset}
              onDeclare={onDeclarePreset}
            />
          ) : null}

          {/* A preset is declared by its own button; the form only
                  replaces one already declared. */}
          {row.subject.kind !== "preset" || target?.kind === "agent" ? (
            <ProviderForm
              key={`${row.id}:${formKey(target)}`}
              target={target}
              initialKind={held.kind}
              working={working === "saving"}
              failure={formFailure}
              onSaveProvider={onSaveProvider}
              onSaveAgent={onSaveAgent}
              onDone={() => retarget(null)}
              onListModels={onListDraftModels}
            />
          ) : null}
        </ProviderDetail>
      )}

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

/** One provider's page: a way back, what it is, then what it holds. */
function ProviderDetail({
  row,
  backRef,
  onBack,
  failureAlert,
  children,
}: {
  row: ProviderRow
  backRef: React.Ref<HTMLButtonElement>
  onBack: () => void
  failureAlert: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-4">
      <Button
        ref={backRef}
        variant="ghost"
        size="sm"
        className="self-start"
        onClick={onBack}
      >
        <HugeiconsIcon
          icon={ArrowLeft01Icon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        All providers
      </Button>
      <div className="flex items-center gap-3">
        <ProviderLogoTile logo={row.logo} />
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h2 id="ai-providers-title" className="text-base font-medium">
            {row.title}
          </h2>
          <p className="truncate text-xs text-muted-foreground">{row.line}</p>
        </div>
        <ProviderStatusPill status={row.status} />
      </div>
      {failureAlert}
      {children}
    </div>
  )
}
