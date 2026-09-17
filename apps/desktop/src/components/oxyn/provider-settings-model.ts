// What the provider settings screen decides before anything leaves it.
//
// The authority on a declaration's validity is the domain
// (`AiProviderConfig::validate`, called by the backend before any write). What
// is judged here is only what must be said **while typing**: a password pasted
// in the endpoint produces a message at once, not when the form is complete
// (docs/UX-SPEC.md, « Configuration des fournisseurs »).

import type { ProviderKind, ProviderReach } from "@/lib/ipc/ai"

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
 * Splits a command line typed by the user into arguments.
 *
 * Whitespace separates; single or double quotes group. No expansion of any
 * kind: the backend runs the program without a shell.
 */
export function parseArguments(line: string): Array<string> {
  const args: Array<string> = []
  let current = ""
  let quote: string | null = null
  let started = false
  for (const char of line) {
    if (quote) {
      if (char === quote) quote = null
      else current += char
      continue
    }
    if (char === '"' || char === "'") {
      quote = char
      started = true
      continue
    }
    if (/\s/.test(char)) {
      if (started) args.push(current)
      current = ""
      started = false
      continue
    }
    current += char
    started = true
  }
  if (started) args.push(current)
  return args
}

export function removalQuestion(label: string) {
  return `Remove the “${label}” declaration?`
}
