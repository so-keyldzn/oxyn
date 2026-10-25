// The rows of the AI providers list, built from what is declared and found.
//
// One row per destination the screen can declare: each agent preset, each
// provider family the form offers (and a family no longer offered that still
// has a declaration), and agents declared by hand. A row says only what the
// backend knows: a declaration, a detection, the last model listing. Nothing
// here says « signed in » — no command reports an agent's sign-in — and no
// model is named unless the provider listed it (docs/UX-SPEC.md, « Oxyn
// carries no list of models »).

import type { PresetState, PresetStates } from "@/components/oxyn/agent-presets"
import { KINDS } from "@/components/oxyn/provider-settings-model"
import type { ModelsState } from "@/components/oxyn/provider-settings-model"
import type {
  AgentPresetDraft,
  DeclaredProvider,
  ExternalAgent,
  ProviderKind,
} from "@/lib/ipc/ai"

export type RowTone = "success" | "warning" | "neutral"

export interface RowStatus {
  label: string
  tone: RowTone
}

/** What `ProviderLogo` draws: the brand's mark, or the icon of what it is. */
export interface RowLogo {
  brand: string
  fallback: "endpoint" | "agent"
}

export type RowSubject =
  | { kind: "preset"; preset: AgentPresetDraft }
  | { kind: "family"; family: ProviderKind }
  | { kind: "custom-agent" }

export interface ProviderRow {
  /** Stable across renders: addresses the row, never rendered. */
  id: string
  title: string
  logo: RowLogo
  /** Its models depend on an endpoint the user points it at. */
  endpointDependent: boolean
  /** One muted line: what the provider listed, or what the row is. */
  line: string
  status: RowStatus
  /** At least one declaration lives under this row. */
  declared: boolean
  subject: RowSubject
}

/** How many listed models the line names before « +N ». */
const NAMED_MODELS = 3

const STATUS = {
  connected: { label: "Connected", tone: "success" },
  declared: { label: "Declared", tone: "success" },
  checking: { label: "Checking status…", tone: "neutral" },
  notConnected: { label: "Not connected", tone: "neutral" },
  failed: { label: "Connection failed", tone: "warning" },
  notFound: { label: "Not found on this machine", tone: "warning" },
  detectionFailed: { label: "Detection failed", tone: "warning" },
} as const satisfies Record<string, RowStatus>

const FAMILIES: Record<ProviderKind, { title: string; description: string }> = {
  anthropic: {
    title: "Anthropic API",
    description: "Hosted API, with your own Anthropic key",
  },
  openai: {
    title: "OpenAI API",
    description: "Hosted API, with your own OpenAI key",
  },
  gemini: {
    title: "Google Gemini API",
    description: "Hosted API, with your own Google key",
  },
  openai_compatible: {
    title: "OpenAI-compatible endpoint",
    description: "Local or self-hosted endpoint: Ollama, LM Studio, Azure…",
  },
}

export function familyTitle(family: ProviderKind) {
  return FAMILIES[family].title
}

function namedModels(listed: ModelsState & { status: "ready" }) {
  const { models } = listed
  if (models.length === 0) return "Lists no model"
  const names = models
    .slice(0, NAMED_MODELS)
    .map((model) => model.displayName || model.id)
    .join(" · ")
  const more = models.length - NAMED_MODELS
  return more > 0 ? `${names} · +${more}` : names
}

/** The worst state first: a failure is not hidden behind a success. */
function familyStatus(states: ReadonlyArray<ModelsState | undefined>) {
  if (states.some((state) => state?.status === "error")) return STATUS.failed
  if (states.some((state) => state?.status === "loading"))
    return STATUS.checking
  if (states.some((state) => state?.status === "ready")) return STATUS.connected
  return STATUS.declared
}

function familyRow(
  family: ProviderKind,
  providers: ReadonlyArray<DeclaredProvider>,
  models: Readonly<Record<string, ModelsState>>
): ProviderRow {
  const { title, description } = FAMILIES[family]
  const states = providers.map((provider) => models[provider.id])
  const listed = states.find((state) => state?.status === "ready")
  const first = providers[0]
  let line = description
  if (listed?.status === "ready") line = namedModels(listed)
  else if (providers.length > 1)
    line = `${providers.length} declared: ${providers.map((p) => p.label).join(", ")}`
  // The default model is the one the user declared, not one Oxyn names.
  else if (first) line = `${first.label} · ${first.model}`
  return {
    id: `family:${family}`,
    title,
    logo: { brand: family, fallback: "endpoint" },
    endpointDependent: family === "openai_compatible",
    line,
    status: first ? familyStatus(states) : STATUS.notConnected,
    declared: first !== undefined,
    subject: { kind: "family", family },
  }
}

function presetStatus(state: PresetState | undefined): RowStatus {
  switch (state?.status) {
    case "detecting":
      return STATUS.checking
    case "error":
      return STATUS.detectionFailed
    case "ready":
      return state.draft.launcher === null
        ? STATUS.notFound
        : STATUS.notConnected
    default:
      return STATUS.notConnected
  }
}

function presetRow(
  preset: AgentPresetDraft,
  state: PresetState | undefined,
  agents: ReadonlyArray<ExternalAgent>
): ProviderRow {
  const declared = agents.some((agent) => agent.preset === preset.id)
  return {
    id: `preset:${preset.id}`,
    title: preset.label,
    logo: { brand: preset.id, fallback: "agent" },
    endpointDependent: false,
    line: "External agent · answers with your own subscription",
    status: declared ? STATUS.declared : presetStatus(state),
    declared,
    subject: { kind: "preset", preset },
  }
}

function customAgentRow(agents: ReadonlyArray<ExternalAgent>): ProviderRow {
  const custom = agents.filter((agent) => agent.preset === null)
  return {
    id: "custom-agent",
    title: "Custom agent",
    logo: { brand: "custom-agent", fallback: "agent" },
    endpointDependent: false,
    line:
      custom.length > 0
        ? custom.map((agent) => agent.label).join(", ")
        : "A program of your own, launched without a shell",
    status: custom.length > 0 ? STATUS.declared : STATUS.notConnected,
    declared: custom.length > 0,
    subject: { kind: "custom-agent" },
  }
}

export function providerRows({
  providers,
  agents,
  presets,
  presetStates,
  models,
}: {
  providers: ReadonlyArray<DeclaredProvider>
  agents: ReadonlyArray<ExternalAgent>
  presets: ReadonlyArray<AgentPresetDraft>
  presetStates: PresetStates
  models: Readonly<Record<string, ModelsState>>
}): Array<ProviderRow> {
  const families = KINDS.filter(
    ({ kind, offered }) =>
      offered || providers.some((provider) => provider.kind === kind)
  ).map(({ kind }) =>
    familyRow(
      kind,
      providers.filter((provider) => provider.kind === kind),
      models
    )
  )
  return [
    ...presets.map((preset) =>
      presetRow(preset, presetStates[preset.id], agents)
    ),
    ...families,
    customAgentRow(agents),
  ]
}
