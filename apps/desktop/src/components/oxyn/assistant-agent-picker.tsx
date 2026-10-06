import * as React from "react"
import { ArrowReloadHorizontalIcon } from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

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
import { SQL_AGENT_ID } from "@/features/assistant/agent-options"
import type { AgentOption, MissingAgent } from "@/lib/ipc/ai"

export interface AssistantAgentPickerProps {
  agents: ReadonlyArray<AgentOption>
  value: string | null
  missingAgent?: MissingAgent | null
  loading?: boolean
  unavailable?: boolean
  disabled?: boolean
  /** Creates a new conversation; never changes the current one's role. */
  onSelect: (id: string) => void
  /**
   * Reads the user's agent files again. A running conversation keeps the
   * prompt it started with (ADR-0049 § 6); only the list changes.
   */
  onReload?: () => void
  reloading?: boolean
  /** Why the last reload failed; the list shown is the one before it. */
  reloadError?: string | null
}

export function AssistantAgentPicker({
  agents,
  value,
  missingAgent = null,
  loading = false,
  unavailable = false,
  disabled = false,
  onSelect,
  onReload,
  reloading = false,
  reloadError = null,
}: AssistantAgentPickerProps) {
  const hintId = React.useId()
  const selectedId = missingAgent ? SQL_AGENT_ID : (value ?? SQL_AGENT_ID)
  const selected = agents.find((agent) => agent.id === selectedId)
  const ordered = [...agents].sort(
    (a, b) =>
      Number(a.origin === "user") - Number(b.origin === "user") ||
      a.name.localeCompare(b.name)
  )

  return (
    <div
      className="flex min-w-0 flex-col gap-2"
      data-slot="assistant-agent-picker"
    >
      <div className="flex min-w-0 items-center gap-1">
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
          >
            <SelectValue>
              <span className="truncate">
                {selected?.name ??
                  (value ? "Recorded agent" : "Choose an agent")}
              </span>
              {selected?.origin === "user" ? (
                <Badge variant="outline">User</Badge>
              ) : null}
            </SelectValue>
          </SelectTrigger>
          <SelectContent alignItemWithTrigger={false}>
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
                  size="icon-sm"
                  aria-label="Reload agents"
                  disabled={reloading}
                  onClick={onReload}
                />
              }
            >
              <HugeiconsIcon icon={ArrowReloadHorizontalIcon} strokeWidth={2} />
            </TooltipTrigger>
            <TooltipContent>Reload agents</TooltipContent>
          </Tooltip>
        ) : null}
      </div>
      <p id={hintId} className="text-xs text-muted-foreground">
        {loading
          ? "Loading agents…"
          : reloading
            ? "Reloading agents…"
            : unavailable
              ? "Agents could not be listed. You can start a new conversation with SQL."
              : agents.length === 0
                ? "No agents are available for this connection."
                : "Choosing another agent starts a new conversation."}
      </p>
      {missingAgent ? (
        <Alert role="status">
          <AlertDescription>
            {missingAgent.name} is no longer available. This conversation
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
    </div>
  )
}
