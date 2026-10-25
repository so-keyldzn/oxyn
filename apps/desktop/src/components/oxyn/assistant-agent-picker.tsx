import * as React from "react"
import {
  ArrowDown01Icon,
  ArrowReloadHorizontalIcon,
  SparklesIcon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import {
  CHIP_CHEVRON,
  CHIP_LIST,
  COMPOSER_CHIP,
  chipLabel,
} from "@/components/oxyn/assistant-composer-chip"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  SQL_AGENT_ID,
  defaultAgentId,
} from "@/features/assistant/agent-options"
import type { AgentOption, MissingAgent } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

export interface AssistantAgentPickerProps {
  agents: ReadonlyArray<AgentOption>
  value: string | null
  /**
   * The agent a blank thread runs, when the panel knows it for the
   * destination the question would go to; else `defaultAgentId(agents)`.
   */
  defaultAgent?: string
  missingAgent?: MissingAgent | null
  loading?: boolean
  unavailable?: boolean
  disabled?: boolean
  /** Creates a new conversation; never changes the current one's role. */
  onSelect: (id: string) => void
  /**
   * Reads the user's agent files again. A question already running keeps
   * the agent it started with; a conversation's next question reads the new
   * files, and one whose file is gone says so (ADR-0049 § 6).
   */
  onReload?: () => void
  reloading?: boolean
  /** Why the last reload failed; the list shown is the one before it. */
  reloadError?: string | null
}

/** What the picker says under it, from what it knows of its list. */
function hintOf({
  agents,
  loading = false,
  reloading = false,
  unavailable = false,
}: AssistantAgentPickerProps) {
  if (loading) return { text: "Loading agents…", steady: false }
  if (reloading) return { text: "Reloading agents…", steady: false }
  if (unavailable)
    return {
      text: "Agents could not be listed. You can start a new conversation with SQL.",
      steady: false,
    }
  if (agents.length === 0)
    return {
      text: "No agents are available for this connection.",
      steady: false,
    }
  return {
    text: "Choosing another agent starts a new conversation.",
    steady: true,
  }
}

/**
 * What the picker has to say beyond its choice: a list that is not there yet
 * or failed, the agent a conversation lost, a reload that failed. Drawn under
 * the picker, or above the question field when the picker is a chip there.
 */
export function AssistantAgentPickerNotes(
  props: AssistantAgentPickerProps & {
    /** Repeats the steady hint, which a chip only gives to a screen reader. */
    withSteadyHint?: boolean
    hintId?: string
  }
) {
  const { missingAgent = null, reloadError = null, withSteadyHint } = props
  const hint = hintOf(props)
  return (
    <>
      {withSteadyHint || !hint.steady ? (
        <p id={props.hintId} className="text-xs text-muted-foreground">
          {hint.text}
        </p>
      ) : null}
      {missingAgent ? (
        <Alert role="status">
          {/* Not `missingAgent.name`: it carries the recorded agent's id,
              since a file that is gone can no longer say its name
              (ADR-0049 § 10). A UUID is not a name to show. */}
          <AlertDescription>
            The agent this conversation used is no longer available — it
            continues with the SQL agent.
          </AlertDescription>
        </Alert>
      ) : null}
      {reloadError ? (
        <Alert variant="destructive">
          <AlertDescription>
            Agents could not be reloaded: {reloadError}
          </AlertDescription>
        </Alert>
      ) : null}
    </>
  )
}

export function AssistantAgentPicker(
  props: AssistantAgentPickerProps & {
    /**
     * Drawn as a chip of the composer's bottom bar: the select and its reload
     * alone, the notes left to `AssistantAgentPickerNotes` above the field.
     */
    chip?: boolean
  }
) {
  const {
    agents,
    value,
    defaultAgent,
    missingAgent = null,
    loading = false,
    disabled = false,
    onSelect,
    onReload,
    reloading = false,
    chip = false,
  } = props
  const hintId = React.useId()
  // SQL is selected in a blank thread, whatever `ordered` lists first
  // (ADR-0055): the display order never decides the default. A session
  // that speaks another language selects the agent the backend runs there
  // (ADR-0065).
  const selectedId = missingAgent
    ? SQL_AGENT_ID
    : (value ?? defaultAgent ?? defaultAgentId(agents))
  const selected = agents.find((agent) => agent.id === selectedId)
  const ordered = [...agents].sort(
    (a, b) =>
      Number(a.origin === "user") - Number(b.origin === "user") ||
      a.name.localeCompare(b.name)
  )

  const hint = hintOf(props)

  const control = (
    <div className={cn("flex min-w-0 items-center", chip ? "gap-0" : "gap-1")}>
      <Select
        items={ordered.map((agent) => ({
          value: agent.id,
          label: agent.name,
        }))}
        value={selected?.id ?? null}
        disabled={disabled || loading || agents.length === 0}
        onValueChange={(id) => {
          if (typeof id !== "string" || id === selectedId) return
          const agent = agents.find((option) => option.id === id)
          if (agent && agent.error === null) onSelect(id)
        }}
      >
        <SelectTrigger
          size="sm"
          aria-label="Agent role"
          aria-describedby={hintId}
          title={chip ? hint.text : undefined}
          className={cn(chip && cn(COMPOSER_CHIP, "max-w-40"))}
        >
          {chip ? (
            <HugeiconsIcon icon={SparklesIcon} strokeWidth={2} aria-hidden />
          ) : null}
          <SelectValue className="min-w-0">
            <span className={chip ? chipLabel(true) : "truncate"}>
              {selected?.name ?? (value ? "Recorded agent" : "Choose an agent")}
            </span>
            {selected?.origin === "user" ? (
              <Badge
                variant="outline"
                className={cn(chip && "hidden @xs/composer:inline-flex")}
              >
                User
              </Badge>
            ) : null}
          </SelectValue>
          {chip ? (
            <HugeiconsIcon
              icon={ArrowDown01Icon}
              strokeWidth={2}
              className={CHIP_CHEVRON}
              aria-hidden
            />
          ) : null}
        </SelectTrigger>
        <SelectContent
          alignItemWithTrigger={false}
          className={cn(chip && CHIP_LIST)}
        >
          <SelectGroup>
            {ordered.map((agent) => (
              <SelectItem
                key={agent.id}
                value={agent.id}
                disabled={agent.error !== null}
              >
                <span className="flex max-w-72 flex-col gap-1">
                  <span className="flex items-center gap-2">
                    {agent.name}
                    {agent.origin === "user" ? (
                      <Badge variant="outline">User</Badge>
                    ) : null}
                  </span>
                  <span className="text-xs whitespace-normal text-muted-foreground">
                    {agent.error ?? agent.description}
                  </span>
                </span>
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      {onReload ? (
        // Inert while a reload runs, not hidden: the button keeps its place
        // and its focus, and a second click cannot queue a second read.
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size={chip ? "icon-xs" : "icon-sm"}
                aria-label="Reload agents"
                disabled={reloading}
                onClick={onReload}
                className={cn(chip && "text-muted-foreground")}
              />
            }
          >
            <HugeiconsIcon icon={ArrowReloadHorizontalIcon} strokeWidth={2} />
          </TooltipTrigger>
          <TooltipContent>Reload agents</TooltipContent>
        </Tooltip>
      ) : null}
    </div>
  )

  if (chip)
    return (
      <span data-slot="assistant-agent-picker" className="inline-flex min-w-0">
        {control}
        <span id={hintId} className="sr-only">
          {hint.text}
        </span>
      </span>
    )

  return (
    <div
      className="flex min-w-0 flex-col gap-2"
      data-slot="assistant-agent-picker"
    >
      {control}
      <AssistantAgentPickerNotes {...props} withSteadyHint hintId={hintId} />
    </div>
  )
}
