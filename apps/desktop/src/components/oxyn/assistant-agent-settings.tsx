import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { MoreHorizontalIcon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { AgentSettings } from "@/features/assistant/transcript"
import type { AgentChoice, AgentOption } from "@/lib/ipc/ai"

/**
 * What the user asked to change, in the agent's own ids.
 *
 * Named an intent, not a change: `lib/ipc/ai.ts` has an `AgentSettingChange`
 * of a flatter shape, which `features/assistant/agent-settings.ts` builds
 * from this one. Two types of one name and two shapes would be a trap.
 *
 * Two targets, because the state has two: the agent's `modes` with its
 * `currentMode`, and its `options`, each named by its own `id`. A select sends
 * the chosen choice's id, a boolean its new state. Ids go back to the agent and
 * nowhere else — none is ever rendered.
 */
export type AgentSettingIntent =
  | { kind: "mode"; mode: string }
  | { kind: "select"; option: string; choice: string }
  | { kind: "boolean"; option: string; on: boolean }

/** Said instead of an id when the agent's current value is not in its list. */
const UNLISTED = "Unlisted value"

type SelectOption = AgentOption & {
  value: Extract<AgentOption["value"], { type: "select" }>
}

function labelOf(choices: ReadonlyArray<AgentChoice>, id: string | null) {
  if (id === null) return "None"
  // Never the id itself: it is the agent's key, not a word for the user, and a
  // value outside the list is said as such rather than shown raw or replaced
  // by the first choice, which would be a lie about what the agent uses.
  return choices.find((choice) => choice.id === id)?.name ?? UNLISTED
}

function messageOf(error: unknown) {
  if (error instanceof Error) return error.message
  if (typeof error === "string") return error
  return "The agent did not say why."
}

/**
 * A disabled control receives no pointer event: the reason sits on a wrapper.
 *
 * The wrapper is **always** there, and only the tooltip switches on. Adding it
 * when a change starts would remount the control under it — and throw the
 * focus of someone who just chose with the keyboard back to the page.
 */
function WithReason({
  reason,
  children,
}: {
  reason: string | null
  children: React.ReactNode
}) {
  return (
    <Tooltip disabled={reason === null}>
      <TooltipTrigger render={<span className="inline-flex min-w-0" />}>
        {children}
      </TooltipTrigger>
      {reason !== null ? (
        <TooltipContent className="max-w-72">{reason}</TooltipContent>
      ) : null}
    </Tooltip>
  )
}

function ChoiceText({ choice }: { choice: AgentChoice }) {
  return (
    // `max-w-64` leaves room, inside a `max-w-80` list, for the item's own
    // padding and check mark: the item's text does not shrink below this.
    <span className="flex max-w-64 min-w-0 flex-col">
      <span dir="auto" className="truncate" title={choice.name}>
        {choice.name}
      </span>
      {choice.description ? (
        <span
          dir="auto"
          className="truncate text-xs text-muted-foreground"
          title={choice.description}
        >
          {choice.description}
        </span>
      ) : null}
    </span>
  )
}

interface ControlProps {
  disabled: boolean
  pending: boolean
  describedBy: string | undefined
}

function ChoiceSelect({
  name,
  choices,
  current,
  disabled,
  pending,
  describedBy,
  onChoose,
}: ControlProps & {
  name: string
  choices: ReadonlyArray<AgentChoice>
  current: string | null
  onChoose: (choice: string) => void
}) {
  const label = labelOf(choices, current)
  return (
    <Select
      value={current}
      items={choices.map((choice) => ({
        value: choice.id,
        label: choice.name,
      }))}
      disabled={disabled}
      onValueChange={(value) => {
        // Controlled on the agent's value: choosing sends, and the trigger keeps
        // showing what the agent last said until it says something else.
        if (typeof value === "string" && value !== current) onChoose(value)
      }}
    >
      <SelectTrigger
        size="sm"
        // Name and value together, whole: the visible text is truncated.
        aria-label={`${name}: ${label}`}
        aria-describedby={describedBy}
        aria-busy={pending || undefined}
        title={`${name}: ${label}`}
        className="max-w-40 min-w-0"
      >
        {pending ? <Spinner /> : null}
        <SelectValue className="min-w-0">
          {() => (
            <span dir="auto" className="truncate">
              {label}
            </span>
          )}
        </SelectValue>
      </SelectTrigger>
      {/* As wide as its choices, not as the trigger: the trigger is cut at
          10 rem, and a list that narrow would cut every choice with it. */}
      <SelectContent
        alignItemWithTrigger={false}
        className="w-auto max-w-80 min-w-(--anchor-width)"
      >
        <SelectGroup>
          {choices.map((choice) => (
            <SelectItem key={choice.id} value={choice.id}>
              <ChoiceText choice={choice} />
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  )
}

function OptionSwitch({
  name,
  on,
  disabled,
  pending,
  describedBy,
  onToggle,
}: ControlProps & {
  name: string
  on: boolean
  onToggle: (on: boolean) => void
}) {
  const id = React.useId()
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 px-1 text-xs">
      <Switch
        id={id}
        size="sm"
        checked={on}
        disabled={disabled}
        aria-describedby={describedBy}
        aria-busy={pending || undefined}
        onCheckedChange={(next) => {
          if (next !== on) onToggle(next)
        }}
      />
      <label
        htmlFor={id}
        dir="auto"
        title={name}
        className="max-w-32 truncate text-muted-foreground"
      >
        {name}
      </label>
      {pending ? <Spinner className="size-3.5" /> : null}
    </span>
  )
}

/**
 * The settings of an external agent: model, reasoning level, mode, and what
 * else the agent declares — as the agent last reported them.
 *
 * **Nothing here is optimistic.** A choice sends the change and the control
 * keeps showing the agent's last value, disabled with a spinner, until the
 * command's reply comes back. **The reply ends the wait, and nothing else
 * does**: an unrelated `agentSettings` event arriving first would re-enable the
 * controls while the command is still in flight, and a second click would send
 * it twice. What is *shown* is another matter — whichever of the reply and the
 * events arrived last, which the caller folds into `settings`. A refused change
 * leaves the displayed value untouched, because it was never replaced, and
 * says why.
 *
 * **One change at a time.** While one waits, every control waits: two changes
 * in flight would let the first answer re-enable a control whose own command
 * has not come back, and a second click would send it twice.
 *
 * Every word comes from the agent — names, descriptions — and is rendered as
 * text. Ids are sent back and never shown. `null` renders nothing at all: an
 * agent that declares no settings has none, and a placeholder would suggest
 * otherwise.
 */
export function AssistantAgentSettings({
  settings,
  disabledReason = null,
  onChange,
}: {
  /**
   * The whole state as last received; replaced, never merged. Before any
   * question, what the agent declared when the panel started it.
   */
  settings: AgentSettings | null
  /** Why nothing can change now — a question running, for one. */
  disabledReason?: string | null
  /**
   * Sends the change. Resolves once the agent accepted it — the caller having
   * already put the agent's answer in `settings` — and rejects with a sentence
   * that reads after « Model was not changed: ».
   */
  onChange: (change: AgentSettingIntent) => Promise<void>
}) {
  // What waits for the agent's answer. A mode and an option are two targets,
  // so the wait is typed by target rather than keyed on a shared string an
  // agent's option id could collide with.
  const [pending, setPending] = React.useState<
    | { kind: "mode"; name: string }
    | { kind: "option"; option: string; name: string }
    | null
  >(null)
  const [failure, setFailure] = React.useState<string | null>(null)
  const reasonId = React.useId()
  if (settings === null) return null

  const reason =
    disabledReason ??
    (pending
      ? `Waiting for the agent to apply the change to ${pending.name}.`
      : null)
  const describedBy = reason === null ? undefined : reasonId
  const locked = reason !== null

  const send = (name: string, change: AgentSettingIntent) => {
    if (locked) return
    setFailure(null)
    setPending(
      change.kind === "mode"
        ? { kind: "mode", name }
        : { kind: "option", option: change.option, name }
    )
    onChange(change).then(
      // Accepted: the reply carried the agent's settings, which the caller has
      // already put in `settings`. Between two questions no event may ever
      // come, so this is where the wait ends.
      () => setPending(null),
      (error: unknown) => {
        setPending(null)
        setFailure(`${name} was not changed: ${messageOf(error)}`)
      }
    )
  }

  const byCategory = (category: AgentOption["category"]) =>
    settings.options.filter((option) => option.category === category)
  const models = byCategory("model")
  const secondary = [...byCategory("thoughtLevel"), ...byCategory("mode")]
  const extra = [...byCategory("modelConfig"), ...byCategory("other")]
  const hasModes = settings.modes.length > 0

  const control = (option: AgentOption) => {
    const common = {
      disabled: locked,
      pending: pending?.kind === "option" && pending.option === option.id,
      describedBy,
    }
    return (
      <WithReason key={option.id} reason={reason}>
        {option.value.type === "select" ? (
          <ChoiceSelect
            {...common}
            name={option.name}
            choices={option.value.choices}
            current={option.value.current}
            onChoose={(choice) =>
              send(option.name, {
                kind: "select",
                option: option.id,
                choice,
              })
            }
          />
        ) : (
          <OptionSwitch
            {...common}
            name={option.name}
            on={option.value.on}
            onToggle={(on) =>
              send(option.name, {
                kind: "boolean",
                option: option.id,
                on,
              })
            }
          />
        )}
      </WithReason>
    )
  }

  const modesControl = hasModes ? (
    <WithReason key="modes" reason={reason}>
      <ChoiceSelect
        disabled={locked}
        pending={pending?.kind === "mode"}
        describedBy={describedBy}
        name="Mode"
        choices={settings.modes}
        current={settings.currentMode}
        onChoose={(mode) => send("Mode", { kind: "mode", mode })}
      />
    </WithReason>
  ) : null

  const more = (inMenu: ReadonlyArray<AgentOption>, withModes: boolean) =>
    inMenu.length > 0 || withModes ? (
      <WithReason reason={reason}>
        <MoreMenu
          options={inMenu}
          modes={withModes ? settings : null}
          disabled={locked}
          describedBy={describedBy}
          onSend={send}
        />
      </WithReason>
    ) : null

  return (
    // A container, so the narrow layout follows the panel's width, not the
    // window's: the same panel is a 240 px column or the whole screen.
    <div data-slot="assistant-agent-settings" className="@container min-w-0">
      <div
        role="group"
        aria-label="Agent settings"
        aria-describedby={describedBy}
        className="flex min-w-0 flex-wrap items-center gap-1"
      >
        {models.map(control)}
        {/* Wide: reasoning and mode inline. Narrow: only the model stays. */}
        <span className="hidden min-w-0 flex-wrap items-center gap-1 @sm:inline-flex">
          {secondary.map(control)}
          {modesControl}
        </span>
        {/* `modelConfig` and `other` stay behind « More » at every width:
            thirty-two selectors inline would push the question field off the
            panel. */}
        <span className="hidden @sm:inline-flex">{more(extra, false)}</span>
        <span className="inline-flex @sm:hidden">
          {more([...secondary, ...extra], hasModes)}
        </span>
      </div>
      {reason !== null ? (
        <span id={reasonId} className="sr-only">
          {reason}
        </span>
      ) : null}
      {failure !== null ? (
        <p
          role="alert"
          data-selectable
          className="mt-1 text-xs wrap-break-word text-destructive"
        >
          {failure}
        </p>
      ) : null}
    </div>
  )
}

function MoreMenu({
  options,
  modes,
  disabled,
  describedBy,
  onSend,
}: {
  options: ReadonlyArray<AgentOption>
  modes: AgentSettings | null
  disabled: boolean
  describedBy: string | undefined
  onSend: (name: string, change: AgentSettingIntent) => void
}) {
  const selects = options.filter(
    (option): option is SelectOption => option.value.type === "select"
  )
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled}
        aria-describedby={describedBy}
        render={
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="More agent settings"
          />
        }
      >
        <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-w-80 min-w-48">
        <DropdownMenuGroup>
          {modes ? (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <span className="min-w-0 flex-1 truncate">Mode</span>
                <span
                  dir="auto"
                  className="max-w-24 truncate text-xs text-muted-foreground"
                >
                  {labelOf(modes.modes, modes.currentMode)}
                </span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="max-w-80">
                <DropdownMenuRadioGroup
                  value={modes.currentMode}
                  onValueChange={(value) => {
                    if (
                      typeof value === "string" &&
                      value !== modes.currentMode
                    )
                      onSend("Mode", { kind: "mode", mode: value })
                  }}
                >
                  {modes.modes.map((mode) => (
                    <DropdownMenuRadioItem key={mode.id} value={mode.id}>
                      <ChoiceText choice={mode} />
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          ) : null}
          {options.map((option) =>
            option.value.type === "boolean" ? (
              <DropdownMenuCheckboxItem
                key={option.id}
                checked={option.value.on}
                onCheckedChange={(on) => {
                  if (option.value.type === "boolean" && on !== option.value.on)
                    onSend(option.name, {
                      kind: "boolean",
                      option: option.id,
                      on,
                    })
                }}
              >
                <span dir="auto" className="truncate" title={option.name}>
                  {option.name}
                </span>
              </DropdownMenuCheckboxItem>
            ) : null
          )}
          {selects.map((option) => (
            <DropdownMenuSub key={option.id}>
              <DropdownMenuSubTrigger>
                <span
                  dir="auto"
                  className="min-w-0 flex-1 truncate"
                  title={option.name}
                >
                  {option.name}
                </span>
                <span
                  dir="auto"
                  className="max-w-24 truncate text-xs text-muted-foreground"
                >
                  {labelOf(option.value.choices, option.value.current)}
                </span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="max-w-80">
                <DropdownMenuRadioGroup
                  value={option.value.current}
                  onValueChange={(value) => {
                    if (
                      typeof value === "string" &&
                      value !== option.value.current
                    )
                      onSend(option.name, {
                        kind: "select",
                        option: option.id,
                        choice: value,
                      })
                  }}
                >
                  {option.value.choices.map((choice) => (
                    <DropdownMenuRadioItem key={choice.id} value={choice.id}>
                      <ChoiceText choice={choice} />
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
