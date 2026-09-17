// Between the settings selector and the backend: what the user did, as the
// command `ai_set_agent_setting` takes it, and a refusal as a sentence.
//
// The selector speaks in intents that keep the target typed — a mode, a
// select, a switch — so it can tell which control waits. The command takes a
// flatter shape. The translation lives here, tested, rather than bending
// either side to the other.

import type { AgentSettingIntent } from "@/components/oxyn/assistant-agent-settings"
import type { AgentSettingAnswer, AgentSettingChange } from "@/lib/ipc/ai"

export function toAgentSettingChange(
  intent: AgentSettingIntent
): AgentSettingChange {
  switch (intent.kind) {
    case "mode":
      return { mode: intent.mode }
    case "select":
      return { option: intent.option, value: intent.choice }
    case "boolean":
      return { option: intent.option, value: intent.on }
  }
}

type Refusal = Extract<AgentSettingAnswer, { type: "refused" }>

/**
 * A refusal, readable after « Model was not changed: ».
 *
 * Derived from the reason, never the bare code: a number tells the user
 * nothing. The agent's code is added after the sentence when there is one,
 * because it is what someone would search for; the agent's own words never
 * cross the boundary, so they are not shown either.
 */
export function refusalMessage(refusal: Refusal): string {
  switch (refusal.reason) {
    case "questionInProgress":
      return "an answer is still running; change it once the answer is done."
    case "unknownMode":
      return "the agent no longer offers this mode."
    case "unknownOption":
      return "the agent no longer offers this setting."
    case "unknownValue":
      return "the agent no longer offers this value."
    case "byAgent":
      return refusal.code === null
        ? "the agent refused the change."
        : `the agent refused the change (error code ${refusal.code}).`
    case "unknown":
      return "the change was refused for a reason this version of Oxyn cannot read."
  }
}
