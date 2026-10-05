// What the model picker decides without rendering: which entries it offers,
// in which order, and whether the form knows enough to ask for a list.
//
// No model is named here: the list is the provider's, and only the provider
// orders it (docs/UX-SPEC.md, « Configuration des fournisseurs »).

import type {
  ModelChoice,
  ModelListing,
  ModelListingReason,
  ModelProbe,
  ProviderKind,
} from "@/lib/ipc/ai"

/** The list as the form holds it, between two requests. */
export type DraftModels =
  | { status: "idle" }
  | { status: "loading" }
  | ModelListing
  /** The call itself failed: a malformed request, not a provider's answer. */
  | { status: "error"; message: string }

export interface ModelEntry {
  /** The provider's model id: what the form stores. */
  id: string
  /** What the user reads; the id when the provider gives no other name. */
  label: string
  /**
   * `listed` comes from the provider; `current` is the value already in the
   * form; `manual` is what the user typed, offered as an id.
   */
  origin: "listed" | "current" | "manual"
  /** The current value, absent from a list that answered. */
  unavailable: boolean
}

function matches(model: ModelChoice, query: string, providerLabel: string) {
  return [model.id, model.displayName, providerLabel].some((text) =>
    text.toLowerCase().includes(query)
  )
}

/**
 * The entries offered for `query`: the current value first, then the
 * provider's models in the provider's order.
 *
 * What was typed leads, since the first entry is the one Enter picks: the
 * entry carrying it exactly, or else the typed id itself — « vega » then Enter
 * keeps « vega », never the « model-vega-mini » it happens to match.
 *
 * The current value is never dropped: a model the provider no longer lists is
 * shown, flagged, and stays selected until the user picks another.
 */
export function modelEntries(
  models: ReadonlyArray<ModelChoice> | null,
  current: string,
  query: string,
  providerLabel: string
): Array<ModelEntry> {
  const wanted = query.trim().toLowerCase()
  const listed = models ?? []
  const entries: Array<ModelEntry> = []
  const selected = listed.find((model) => model.id === current)
  if (current !== "") {
    const entry: ModelEntry = selected
      ? {
          id: selected.id,
          label: selected.displayName,
          origin: "listed",
          unavailable: false,
        }
      : {
          id: current,
          label: current,
          origin: "current",
          unavailable: models !== null,
        }
    if (
      wanted === "" ||
      (selected
        ? matches(selected, wanted, providerLabel)
        : current.toLowerCase().includes(wanted))
    )
      entries.push(entry)
  }
  for (const model of listed) {
    if (model.id === current) continue
    if (wanted !== "" && !matches(model, wanted, providerLabel)) continue
    entries.push({
      id: model.id,
      label: model.displayName,
      origin: "listed",
      unavailable: false,
    })
  }
  const typed = query.trim()
  if (typed === "") return entries
  const exact = entries.find((entry) => entry.id === typed)
  if (exact) return [exact, ...entries.filter((entry) => entry !== exact)]
  return [
    { id: typed, label: typed, origin: "manual", unavailable: false },
    ...entries,
  ]
}

/** The values of the form a list depends on. */
export interface ProbeInputs {
  kind: ProviderKind
  endpoint: string
  key: string
  /** The declaration being edited, if any. */
  editing: { id: string; endpoint: string; keyConfigured: boolean } | null
}

/** A host that resolves to this machine by its name alone. */
export function isLocalEndpoint(endpoint: string): boolean {
  const match = /^[a-z][a-z0-9+.-]*:\/\/(\[[^\]]*\]|[^/:?#]*)/i.exec(
    endpoint.trim()
  )
  const host = (match?.[1] ?? "").toLowerCase()
  return (
    host === "localhost" ||
    host.endsWith(".localhost") ||
    host === "[::1]" ||
    /^127(?:\.\d{1,3}){3}$/.test(host)
  )
}

/**
 * What the form can ask a list for, or `null` while a field is missing.
 *
 * `strict` is the automatic listing: it waits for a key, unless the endpoint
 * is local or the declaration being edited keeps its stored one — the backend
 * reuses a stored key only for the stored endpoint. A click on « Refresh
 * models » is not strict: the provider then says itself that a key is missing.
 */
export function probeOf(
  inputs: ProbeInputs,
  strict: boolean
): ModelProbe | null {
  const endpoint = inputs.endpoint.trim()
  const { editing } = inputs
  if (endpoint === "" && editing === null) return null
  const key = inputs.key.trim() === "" ? null : inputs.key
  const storedKey =
    editing !== null &&
    editing.keyConfigured &&
    (endpoint === "" || endpoint === editing.endpoint)
  if (
    strict &&
    key === null &&
    !storedKey &&
    !isLocalEndpoint(endpoint === "" ? (editing?.endpoint ?? "") : endpoint)
  )
    return null
  return {
    id: editing?.id ?? null,
    kind: inputs.kind,
    baseUrl: endpoint,
    key,
  }
}

const REASONS: Record<ModelListingReason, string> = {
  unauthorized: "The provider refused the API key.",
  forbidden: "This API key may not list models.",
  unsupported:
    "This endpoint does not list its models: type the model id instead.",
  rateLimited: "The provider is limiting requests: try again shortly.",
  timeout: "The provider did not answer in time.",
  unreachable: "The endpoint could not be reached.",
  malformed: "The provider's answer could not be read.",
  invalidEndpoint: "The endpoint is not a valid URL.",
  missingKey: "This provider needs an API key to list its models.",
}

/** The sentence for a reason; one this build does not know reads generically. */
export function reasonCopy(reason: ModelListingReason): string {
  return Object.hasOwn(REASONS, reason)
    ? REASONS[reason]
    : "The models could not be listed."
}
