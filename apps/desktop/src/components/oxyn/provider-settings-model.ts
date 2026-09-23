// What the provider settings screen decides before anything leaves it.
//
// The authority on a declaration's validity is the domain
// (`AiProviderConfig::validate`, called by the backend before any write). What
// is judged here is only what must be said **while typing**: a password pasted
// in the endpoint produces a message at once, not when the form is complete
// (docs/UX-SPEC.md, « Configuration des fournisseurs »).

import type { ModelChoice, ProviderKind, ProviderReach } from "@/lib/ipc/ai"

export type ModelsState =
  | { status: "loading" }
  | { status: "ready"; models: ReadonlyArray<ModelChoice> }
  | { status: "error"; message: string }

/** Which write failed: the screen names it, rather than a bare « Failed ». */
export type SettingsOperation =
  | "save-provider"
  | "save-agent"
  | "declare-preset"
  | "remove-provider"
  | "remove-agent"

export interface ProviderSettingsFailure {
  operation: SettingsOperation
  message: string
  retryable: boolean
  /** A failed save forgot the key it carried: say so, rather than a blank field. */
  keyMustBeRetyped: boolean
}

export const FAILURE_TITLES: Record<SettingsOperation, string> = {
  "save-provider": "Provider not saved",
  "save-agent": "Agent not saved",
  "declare-preset": "Agent not declared",
  "remove-provider": "Provider not removed",
  "remove-agent": "Agent not removed",
}

/** Whether the failure belongs to the form, where it is shown next to its fields. */
export function isFormFailure(failure: ProviderSettingsFailure | null) {
  return (
    failure?.operation === "save-provider" ||
    failure?.operation === "save-agent"
  )
}

/** « 1 argument », « 3 arguments »: English plurals of this screen only. */
export function counted(count: number, singular: string, plural: string) {
  return `${count} ${count === 1 ? singular : plural}`
}

/** The domain's words, so the screen and the backend refusal read the same. */
export const CREDENTIALS_IN_ENDPOINT =
  "provider base URL must not carry credentials; keep the key in the system keychain and reference it"

export const KINDS: ReadonlyArray<{
  kind: ProviderKind
  label: string
  /** An example, never a value: the field starts empty. */
  placeholder: string
}> = [
  // The first three are the endpoints `oxyn-llm` itself defaults to.
  {
    kind: "anthropic",
    label: "Anthropic",
    placeholder: "https://api.anthropic.com",
  },
  { kind: "openai", label: "OpenAI", placeholder: "https://api.openai.com/v1" },
  {
    kind: "gemini",
    label: "Gemini",
    placeholder: "https://generativelanguage.googleapis.com",
  },
  {
    kind: "openai_compatible",
    label: "OpenAI-compatible (Ollama, LM Studio, Azure…)",
    placeholder: "http://localhost:11434/v1",
  },
]

export function kindLabel(kind: ProviderKind): string {
  return KINDS.find((entry) => entry.kind === kind)?.label ?? kind
}

/** Does this endpoint carry `user:password@` in its authority? */
export function endpointCarriesCredentials(endpoint: string): boolean {
  const match = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(endpoint.trim())
  return match !== null && (match[1] ?? "").includes("@")
}

export const REACH_LABELS: Record<ProviderReach, string> = {
  local: "Local",
  remote: "Remote",
  // Not rounded to either: Oxyn could not classify it, and it counts as remote.
  unresolved: "Unresolved",
}

/** « Local · measured 14:32 »: a classification is dated, never eternal. */
export function reachSummary(reach: ProviderReach, measuredAtMs: number) {
  if (!Number.isFinite(measuredAtMs) || measuredAtMs <= 0) {
    return `${REACH_LABELS[reach]} · measurement time unknown`
  }
  const time = new Date(measuredAtMs).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  })
  return `${REACH_LABELS[reach]} · measured ${time}`
}

/**
 * The arguments typed in the form, **one per line**, exactly as typed.
 *
 * No grammar at all — no quotes, no splitting on spaces (ADR-0026): a grammar
 * of the command line is a surface, and one line per argument has no
 * ambiguous case. Only a trailing carriage return is dropped, and blank lines
 * are skipped.
 */
export function parseArguments(text: string): Array<string> {
  return text
    .split("\n")
    .map((line) => line.replace(/\r$/, ""))
    .filter((line) => line.trim() !== "")
}

/** A variable name a process environment accepts everywhere. */
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/

/**
 * The environment typed in the form, one `NAME=value` per line.
 *
 * The value is everything after the first `=`, as typed. A line without a
 * valid name is an error rather than skipped: a variable the user believes set
 * and that is not is exactly the failure nobody sees.
 */
export function parseEnvironment(
  text: string
):
  | { ok: true; env: Array<{ name: string; value: string }> }
  | { ok: false; line: number } {
  const env: Array<{ name: string; value: string }> = []
  const lines = text.split("\n")
  for (const [index, raw] of lines.entries()) {
    const line = raw.replace(/\r$/, "")
    if (line.trim() === "") continue
    const equals = line.indexOf("=")
    const name = equals < 0 ? "" : line.slice(0, equals)
    if (!ENV_NAME.test(name)) return { ok: false, line: index + 1 }
    env.push({ name, value: line.slice(equals + 1) })
  }
  return { ok: true, env }
}

export function removalQuestion(label: string) {
  return `Remove the “${label}” declaration?`
}
