import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  CheckmarkCircle02Icon,
  InformationCircleIcon,
  Search01Icon,
} from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import type {
  AgentPresetDraft,
  AgentPresetId,
  ExternalAgent,
} from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

/** What « Detect » found for one preset, or why it could not. */
export type PresetState =
  | { status: "idle" }
  | { status: "detecting" }
  | { status: "ready"; draft: AgentPresetDraft }
  | { status: "error"; message: string }

export type PresetStates = Partial<Record<AgentPresetId, PresetState>>

function commandLine(draft: AgentPresetDraft) {
  return [draft.command, ...draft.args].join(" ")
}

/**
 * One program looked for. `needed` says whether its absence stops the agent:
 * the launcher does, the agent's own program does not, since the adapter
 * ships its own copy.
 */
function Found({
  what,
  path,
  needed,
}: {
  what: string
  path: string | null
  needed: boolean
}) {
  const missing = path === null && needed
  return (
    <p className="flex items-start gap-1.5">
      <HugeiconsIcon
        icon={
          path
            ? CheckmarkCircle02Icon
            : needed
              ? Alert02Icon
              : InformationCircleIcon
        }
        strokeWidth={2}
        className={cn(
          "mt-0.5 size-3.5 shrink-0",
          missing && "text-destructive"
        )}
        aria-hidden
      />
      <span className="min-w-0">
        {what}:{" "}
        {path ? (
          <code data-selectable className="font-mono break-all">
            {path}
          </code>
        ) : (
          <span className={cn(missing && "text-destructive")}>
            not found on this machine
          </span>
        )}
      </span>
    </p>
  )
}

/** Declared already, and confined: declaring it again would only duplicate it. */
function isDeclared(
  preset: AgentPresetDraft,
  agents: ReadonlyArray<ExternalAgent>
) {
  return agents.some((agent) => agent.preset === preset.id && agent.confined)
}

/**
 * Ready-made declarations for the agents Oxyn knows.
 *
 * The machine is looked at when the screen opens — a read of the usual
 * directories, nothing run — but nothing is declared here on its own: the user
 * reads what was found and confirms, twice, since declaring a program to run
 * also asks in a system dialog
 * ([ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
 * ADR-0026).
 *
 * The user keeps their own subscription: the agent signs its user in itself,
 * and Oxyn holds no key for it.
 */
export function AgentPresets({
  presets,
  states,
  agents,
  declaring,
  onDetect,
  onDeclare,
}: {
  presets: ReadonlyArray<AgentPresetDraft>
  states: PresetStates
  /** The agents already declared, to say which presets are. */
  agents: ReadonlyArray<ExternalAgent>
  /** The preset being declared, if any. */
  declaring: AgentPresetId | null
  onDetect: (id: AgentPresetId) => void
  onDeclare: (draft: AgentPresetDraft) => void
}) {
  if (presets.length === 0) return null
  return (
    <section
      data-slot="agent-presets"
      aria-label="Agents installed on this machine"
      className="flex flex-col gap-2"
    >
      <h3 className="text-sm font-medium">Already on this machine</h3>
      <p className="text-xs text-muted-foreground">
        These agents answer with your own subscription. Oxyn never sees their
        credentials, and never serves a local-only connection with one.
      </p>
      <div className="flex flex-col gap-2">
        {presets.map((preset) => {
          const state = states[preset.id] ?? { status: "idle" }
          const found = state.status === "ready" ? state.draft : preset
          const missingLauncher =
            state.status === "ready" && found.launcher === null
          const declared = isDeclared(preset, agents)
          // Declared already: its declaration is listed above. Repeating the
          // command and every path found would say it twice.
          if (declared) {
            return (
              <div
                key={preset.id}
                data-slot="agent-preset"
                className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border px-3 py-2 text-xs"
              >
                <span className="text-sm font-medium">{preset.label}</span>
                <Badge variant="secondary">
                  <HugeiconsIcon
                    icon={CheckmarkCircle02Icon}
                    strokeWidth={2}
                    data-icon="inline-start"
                  />
                  Already declared
                </Badge>
                <span className="min-w-0 basis-full text-muted-foreground">
                  Not signed in yet? Run{" "}
                  <code data-selectable className="font-mono break-all">
                    {preset.signIn}
                  </code>{" "}
                  in a terminal.
                </span>
              </div>
            )
          }
          return (
            <div
              key={preset.id}
              data-slot="agent-preset"
              className="flex min-w-0 flex-col gap-2 rounded-lg border p-3 text-xs"
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium">{preset.label}</span>
                <Badge variant="outline" className="font-mono">
                  {preset.package}@{preset.version}
                </Badge>
                <div className="ml-auto flex items-center gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={state.status === "detecting"}
                    onClick={() => onDetect(preset.id)}
                  >
                    {state.status === "detecting" ? (
                      <Spinner data-icon="inline-start" />
                    ) : (
                      <HugeiconsIcon
                        icon={Search01Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                    )}
                    Detect again
                  </Button>
                  <Button
                    size="sm"
                    // Before detection the draft is a bare `npx` without the
                    // `PATH` it needs from the Finder: declaring it would
                    // save an agent that cannot start.
                    disabled={
                      declaring !== null ||
                      state.status !== "ready" ||
                      missingLauncher
                    }
                    onClick={() => onDeclare(found)}
                  >
                    {declaring === preset.id ? (
                      <Spinner data-icon="inline-start" />
                    ) : null}
                    Declare {preset.label}
                  </Button>
                </div>
              </div>

              <code
                data-selectable
                tabIndex={0}
                role="region"
                aria-label={`Command for ${preset.label}`}
                className="overflow-x-auto rounded-md bg-muted/40 px-2 py-1.5 font-mono whitespace-nowrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {commandLine(found)}
              </code>

              {state.status === "ready" ? (
                <div className="flex flex-col gap-1 text-muted-foreground">
                  <Found what="Launcher" path={found.launcher} needed />
                  <Found
                    what={`${preset.label} command`}
                    path={found.agentProgram}
                    needed={false}
                  />
                  {found.env.length > 0 ? (
                    <p className="min-w-0">
                      Environment: {found.env.map((v) => v.name).join(", ")} —
                      set so the agent finds Node when Oxyn is started from the
                      Finder.
                    </p>
                  ) : null}
                  {missingLauncher ? (
                    <p className="text-destructive">
                      Install Node (the adapter needs npx) or point the command
                      at the adapter yourself, then detect again.
                    </p>
                  ) : null}
                  {found.agentProgram === null ? (
                    <p>
                      {preset.label} itself was not found. The adapter ships its
                      own copy, and you sign in with{" "}
                      <code className="font-mono">{preset.signIn}</code>.
                    </p>
                  ) : null}
                </div>
              ) : null}

              {state.status === "error" ? (
                <p role="alert" data-selectable className="text-destructive">
                  {state.message}
                </p>
              ) : null}

              {state.status === "idle" ? (
                <p className="text-muted-foreground">
                  Oxyn looks for the program in the usual places when this
                  screen opens. Nothing is saved until you confirm.
                </p>
              ) : null}

              <p className="text-muted-foreground">
                After declaring, sign in with{" "}
                <code data-selectable className="font-mono">
                  {preset.signIn}
                </code>{" "}
                in a terminal, if you have not already.
              </p>
            </div>
          )
        })}
      </div>
    </section>
  )
}
