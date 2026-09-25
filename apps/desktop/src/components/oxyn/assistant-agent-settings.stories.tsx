import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantAgentSettings } from "./assistant-agent-settings"
import { AssistantView } from "./assistant-view"
import panel from "./assistant-view.stories"
import { assistantState, destinations } from "./assistant-fixtures"
import { expectContainedInFrame, openFrame } from "./frame-overflow"
import { NEW_THREAD } from "@/features/assistant/thread"
import { Button } from "@/components/ui/button"
import type { AgentSettings } from "@/features/assistant/transcript"
import type { AgentChoice, AgentOption } from "@/lib/ipc/ai"

// Ids are shaped so no name could ever contain one: an assertion that no id is
// visible then means something, instead of passing on a coincidence.
function choice(
  id: string,
  name: string,
  description: string | null = null
): AgentChoice {
  return { id: `id-${id}-7f3a`, name, description }
}

function select(
  id: string,
  name: string,
  category: AgentOption["category"],
  choices: Array<AgentChoice>,
  current: string
): AgentOption {
  return {
    id: `opt-${id}-9c1e`,
    name,
    description: null,
    category,
    value: { type: "select", current, choices },
  }
}

function toggle(
  id: string,
  name: string,
  category: AgentOption["category"],
  on: boolean
): AgentOption {
  return {
    id: `opt-${id}-9c1e`,
    name,
    description: null,
    category,
    value: { type: "boolean", on },
  }
}

const SONNET = choice("sonnet", "Sonnet", "Balanced speed and depth")
const OPUS = choice("opus", "Opus", "Deepest reasoning, slower")
const HAIKU = choice("haiku", "Haiku", "Fastest")
const MODEL = select(
  "model",
  "Model",
  "model",
  [SONNET, OPUS, HAIKU],
  SONNET.id
)

const THINKING = select(
  "thinking",
  "Thinking",
  "thoughtLevel",
  [
    choice("off", "Off"),
    choice("low", "Low"),
    choice("high", "High", "Spends more tokens before answering"),
  ],
  "id-low-7f3a"
)

const CLAUDE_LIKE: AgentSettings = {
  modes: [
    choice("default", "Default", "Asks before acting"),
    choice("plan", "Plan", "Plans without acting"),
  ],
  currentMode: "id-default-7f3a",
  options: [MODEL, THINKING],
}

const CODEX_LIKE: AgentSettings = {
  modes: [],
  currentMode: null,
  options: [
    select(
      "model",
      "Model",
      "model",
      [choice("gpt", "GPT-5 Codex"), choice("mini", "GPT-5 mini")],
      "id-gpt-7f3a"
    ),
    select(
      "effort",
      "Reasoning effort",
      "thoughtLevel",
      [choice("minimal", "Minimal"), choice("medium", "Medium")],
      "id-medium-7f3a"
    ),
    select(
      "approval",
      "Approval policy",
      "mode",
      [choice("ask", "Ask"), choice("auto", "Auto")],
      "id-ask-7f3a"
    ),
    toggle("search", "Web search", "other", false),
    select(
      "verbosity",
      "Verbosity",
      "modelConfig",
      [choice("terse", "Terse"), choice("full", "Full")],
      "id-terse-7f3a"
    ),
  ],
}

/** Every id a state carries — what must never reach the screen. */
function idsOf(settings: AgentSettings) {
  const ids = [...settings.modes.map((mode) => mode.id)]
  if (settings.currentMode !== null) ids.push(settings.currentMode)
  for (const option of settings.options) {
    ids.push(option.id)
    if (option.value.type === "select") {
      ids.push(option.value.current)
      ids.push(...option.value.choices.map((item) => item.id))
    }
  }
  return ids
}

/** No id in the visible text, nor in what a pointer or a reader is told. */
async function noIdIsShown(settings: AgentSettings) {
  const onScreen = document.body.innerText
  const told = [...document.body.querySelectorAll("[aria-label],[title]")]
    .flatMap((element) => [
      element.getAttribute("aria-label") ?? "",
      element.getAttribute("title") ?? "",
    ])
    .join("\n")
  for (const id of idsOf(settings)) {
    await expect(onScreen).not.toContain(id)
    await expect(told).not.toContain(id)
  }
}

const body = () => within(document.body)

// Portalled popups animate in: wait for the frame where they are really there.
async function visible(element: HTMLElement) {
  await waitFor(() => expect(element).toBeVisible())
  return element
}

/**
 * Every popup fully gone — Base UI's focus guards included.
 *
 * Axe runs after `play`. A select or a menu still animating out leaves its
 * `aria-hidden` focus guards in the page, which is a state the user is never
 * left in; the same wait `preview-controls.stories.tsx` ends on.
 */
async function settled() {
  await waitFor(() =>
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  )
}

const meta = {
  title: "Oxyn/Assistant/AgentSettings",
  component: AssistantAgentSettings,
  args: {
    settings: CLAUDE_LIKE,
    disabledReason: null,
    onChange: fn(async () => {}),
  },
  decorators: [
    (Story) => (
      <div className="w-[560px] border p-2">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantAgentSettings>

export default meta
type Story = StoryObj<typeof meta>

/**
 * The shape Claude Agent 0.78.0 declares on `session/new`, as measured on
 * 2026-09-23: `model` (ten choices), `effort` (`thought_level`), `fast`
 * (`model_config`). No mode: Oxyn confines it, and the modes are withdrawn
 * from a confined agent (ADR-0032).
 *
 * The categories and the counts are the measurement; the choices' names are
 * stand-ins — they are the adapter's, depend on the account, and are not
 * recorded.
 */
const CLAUDE_MEASURED: AgentSettings = {
  modes: [],
  currentMode: null,
  options: [
    select(
      "model",
      "Model",
      "model",
      Array.from({ length: 10 }, (_, index) =>
        choice(`model-${index + 1}`, `Model ${index + 1}`)
      ),
      "id-model-1-7f3a"
    ),
    select(
      "effort",
      "Effort",
      "thoughtLevel",
      [
        choice("low", "Low"),
        choice("medium", "Medium"),
        choice("high", "High"),
      ],
      "id-medium-7f3a"
    ),
    toggle("fast", "Fast mode", "modelConfig", false),
  ],
}

/** What the panel passes once it started Claude Code, before any question. */
const startedClaude = {
  startup: {
    status: "ready" as const,
    key: "k",
    label: "Claude Code",
    version: "Claude Agent 0.78.0",
    settings: CLAUDE_MEASURED,
  },
  signInStates: {},
  onCancel: fn(),
  onStart: fn(),
  onSignIn: fn(),
  onCopy: fn(() => true),
}

/**
 * Model and effort inline, the fast switch behind « More », and a change
 * made before any question sent as the agent's own ids.
 */
async function everythingClaudeDeclaresIsOffered(
  onChange: (...args: Array<never>) => unknown
) {
  const screen = body()
  await expect(
    screen.getByRole("combobox", { name: "Model: Model 1" })
  ).toBeVisible()
  const effort = screen.getByRole("combobox", { name: "Effort: Medium" })
  await expect(effort).toBeVisible()

  await userEvent.click(
    screen.getByRole("button", { name: "More agent settings" })
  )
  await visible(
    await screen.findByRole("menuitemcheckbox", { name: /Fast mode/ })
  )
  await userEvent.keyboard("{Escape}")
  await settled()

  await userEvent.click(effort)
  await userEvent.click(
    await visible(await screen.findByRole("option", { name: /High/ }))
  )
  await expect(onChange).toHaveBeenCalledWith({
    kind: "select",
    option: "opt-effort-9c1e",
    choice: "id-high-7f3a",
  })
  await settled()
}

/** No settings declared: nothing at all — no skeleton, no reserved room. */
export const Null: Story = {
  args: { settings: null },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
    await expect(
      canvasElement.querySelector('[data-slot="assistant-agent-settings"]')
    ).toBeNull()
  },
}

/**
 * Before the first question: the agent the panel started declared its
 * settings, and they are offered before anything is asked.
 */
export const BeforeTheFirstQuestion: Story = {
  args: { settings: startedClaude.startup.settings },
  play: async ({ args }) => {
    await everythingClaudeDeclaresIsOffered(args.onChange)
  },
}

/**
 * The same, in the whole panel: opened on Claude Code, no question asked, and
 * its model, its effort and its fast switch are already there.
 */
export const InThePanelBeforeTheFirstQuestion: StoryObj<typeof AssistantView> =
  {
    render: (args) => (
      <div className="h-[640px] max-w-2xl border">
        <AssistantView {...args} />
      </div>
    ),
    args: {
      ...panel.args,
      agentStartup: startedClaude,
      state: assistantState(NEW_THREAD),
      selected: destinations[1] ?? null,
      model: null,
      onChangeAgentSetting: fn(async () => {}),
    },
    play: async ({ args, canvasElement }) => {
      await expect(
        within(canvasElement).getByText(/Claude Agent 0\.78\.0 ·/)
      ).toBeVisible()
      await everythingClaudeDeclaresIsOffered(args.onChangeAgentSetting ?? fn())
    },
  }

export const ModelOnly: Story = {
  args: { settings: { modes: [], currentMode: null, options: [MODEL] } },
  play: async ({ args }) => {
    const screen = body()
    const trigger = screen.getByRole("combobox", { name: "Model: Sonnet" })
    await expect(trigger).toHaveTextContent("Sonnet")
    // Nothing else to show: no « More » with an empty menu behind it.
    await expect(
      screen.queryByRole("button", { name: "More agent settings" })
    ).toBeNull()
    await noIdIsShown(args.settings as AgentSettings)
  },
}

/** Model, reasoning and mode — and the whole keyboard path on one of them. */
export const ClaudeLike: Story = {
  play: async ({ args }) => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    await expect(
      screen.getByRole("combobox", { name: "Thinking: Low" })
    ).toBeVisible()
    await expect(
      screen.getByRole("combobox", { name: "Mode: Default" })
    ).toBeVisible()

    // Open with the keyboard, move, choose.
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    const opus = await visible(
      await screen.findByRole("option", { name: /Opus/ })
    )
    await noIdIsShown(args.settings as AgentSettings)
    await userEvent.click(opus)
    await expect(args.onChange).toHaveBeenCalledWith({
      kind: "select",
      option: "opt-model-9c1e",
      choice: "id-opus-7f3a",
    })
    // Not optimistic: until the agent answers, the agent's value stays shown.
    await expect(model).toHaveTextContent("Sonnet")
    await settled()
  },
}

export const EscapeClosesWithoutSending: Story = {
  play: async ({ args }) => {
    const screen = body()
    const thinking = screen.getByRole("combobox", { name: "Thinking: Low" })
    thinking.focus()
    await userEvent.keyboard("{ArrowDown}")
    await visible(await screen.findByRole("listbox"))
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull())
    await expect(args.onChange).not.toHaveBeenCalled()
    await settled()
  },
}

export const CodexLike: Story = {
  args: { settings: CODEX_LIKE },
  play: async ({ args }) => {
    const screen = body()
    await expect(
      screen.getByRole("combobox", { name: "Approval policy: Ask" })
    ).toBeVisible()
    // `modelConfig` and `other` wait behind « More », even at full width.
    await userEvent.click(
      screen.getByRole("button", { name: "More agent settings" })
    )
    await visible(
      await screen.findByRole("menuitemcheckbox", { name: "Web search" })
    )
    await expect(
      screen.getByRole("menuitem", { name: /Verbosity/ })
    ).toBeVisible()
    await noIdIsShown(args.settings as AgentSettings)
    await userEvent.keyboard("{Escape}")
    // Closed for real before axe looks: an animating menu leaves Base UI's
    // focus guards behind, which is not what the user is left with.
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    await expect(args.onChange).not.toHaveBeenCalled()
    await settled()
  },
}

/** Thirty-two options and a hundred and twenty-eight choices, bounded. */
export const ManyChoices: Story = {
  args: {
    settings: {
      modes: [],
      currentMode: null,
      options: [
        select(
          "model",
          "Model",
          "model",
          Array.from({ length: 128 }, (_, index) =>
            choice(`m${index}`, `model-variant-${index}`)
          ),
          "id-m0-7f3a"
        ),
        ...Array.from({ length: 31 }, (_, index) =>
          select(
            `o${index}`,
            `Option ${index}`,
            "other",
            [choice(`a${index}`, "A"), choice(`b${index}`, "B")],
            `id-a${index}-7f3a`
          )
        ),
      ],
    },
  },
  play: async () => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: /^Model:/ })
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    await visible(await screen.findByRole("listbox"))
    await expect(screen.getAllByRole("option")).toHaveLength(128)
    // A list that scrolls, not one that runs off the screen: the popup is the
    // scroller, the listbox inside it is as tall as its hundred-odd rows.
    const popup = document.body.querySelector(
      '[data-slot="select-content"]'
    ) as HTMLElement
    await expect(popup.scrollHeight).toBeGreaterThan(popup.clientHeight)
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull())
    await settled()
  },
}

const LONG = "réflexion-très-approfondie-".repeat(19).slice(0, 512)

export const LongNames: Story = {
  args: {
    settings: {
      modes: [choice("rtl", "وضع التخطيط المفصل", "يخطط قبل التنفيذ")],
      currentMode: "id-rtl-7f3a",
      options: [
        select("model", LONG, "model", [choice("long", LONG)], "id-long-7f3a"),
      ],
    },
  },
  play: async () => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: `${LONG}: ${LONG}` })
    // Truncated on screen, whole for a reader and on hover.
    const text = model.querySelector("[dir=auto]") as HTMLElement
    await expect(text.scrollWidth).toBeGreaterThan(text.clientWidth)
    await expect(model).toHaveAttribute("title", `${LONG}: ${LONG}`)
    await expect(
      screen.getByRole("combobox", { name: "Mode: وضع التخطيط المفصل" })
    ).toBeVisible()
  },
}

const UNBROKEN = "claude_sonnet_analytics_team_staging_inference_profile_v2"

/**
 * Long choices once the list is open: each one is cut with an ellipsis inside
 * the list's frame, never drawn past it.
 */
export const LongChoicesStayInTheList: Story = {
  args: {
    settings: {
      modes: [],
      currentMode: null,
      options: [
        select(
          "model",
          "Model",
          "model",
          [
            choice("long", UNBROKEN, `${UNBROKEN} — ${LONG}`),
            choice("rtl", "وضع التخطيط المفصل", "يخطط قبل التنفيذ"),
            SONNET,
          ],
          SONNET.id
        ),
      ],
    },
  },
  play: async () => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    await visible(await screen.findByRole("listbox"))
    await expectContainedInFrame(await openFrame("select-content"))
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull())
    await settled()
  },
}

/** Chosen and sent: the control waits, shows the agent's value, and is inert. */
export const Pending: Story = {
  args: {
    // Never settles: the agent has not answered yet.
    onChange: fn(() => new Promise<void>(() => {})),
  },
  play: async ({ args }) => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    await userEvent.click(
      await visible(await screen.findByRole("option", { name: /Haiku/ }))
    )
    await expect(args.onChange).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(model).toBeDisabled())
    await expect(model).toHaveAttribute("aria-busy", "true")
    await expect(model).toHaveTextContent("Sonnet")
    // One change at a time: the others wait too, and say why.
    await expect(
      screen.getByRole("combobox", { name: "Thinking: Low" })
    ).toBeDisabled()
    await expect(
      screen.getByText("Waiting for the agent to apply the change to Model.")
    ).toBeInTheDocument()
    await settled()
  },
}

/**
 * The reply ends the wait on its own — between two questions no event may
 * ever come. (The settings it carried are folded upstream, by the store.)
 */
export const ReplyEndsTheWait: Story = {
  args: { onChange: fn(async () => {}) },
  play: async ({ args }) => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    await userEvent.click(
      await visible(await screen.findByRole("option", { name: /Opus/ }))
    )
    await expect(args.onChange).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(model).toBeEnabled())
    await expect(model).not.toHaveAttribute("aria-busy")
    await settled()
  },
}

/**
 * An `agentSettings` event arriving before the reply does **not** end the wait:
 * it would re-enable the controls while the command is still in flight, and a
 * second click would send it twice. The new state is shown all the same.
 */
export const EventBeforeReplyKeepsWaiting: Story = {
  args: { onChange: fn(() => new Promise<void>(() => {})) },
  render: function Harness(args) {
    const [settings, setSettings] = React.useState(CLAUDE_LIKE)
    return (
      <div className="flex flex-col gap-2">
        <AssistantAgentSettings {...args} settings={settings} />
        {/* Story harness: stands in for the agent's event stream. */}
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            setSettings({
              ...CLAUDE_LIKE,
              options: [
                {
                  ...MODEL,
                  value: { ...MODEL.value, current: HAIKU.id },
                } as AgentOption,
                THINKING,
              ],
            })
          }
        >
          Agent sends an event
        </Button>
      </div>
    )
  },
  play: async ({ args }) => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    await userEvent.click(
      await visible(await screen.findByRole("option", { name: /Opus/ }))
    )
    await settled()
    await expect(args.onChange).toHaveBeenCalledTimes(1)
    await userEvent.click(
      screen.getByRole("button", { name: "Agent sends an event" })
    )
    // Shown: the event's state. Still waiting: the reply has not come.
    const updated = await screen.findByRole("combobox", {
      name: "Model: Haiku",
    })
    await expect(updated).toBeDisabled()
    await expect(updated).toHaveAttribute("aria-busy", "true")
  },
}

/** Refused: the agent's value never moved, and the refusal is said. */
export const Failed: Story = {
  args: {
    onChange: fn(async () => {
      throw new Error("Opus is not available on this plan")
    }),
  },
  play: async ({ args }) => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    model.focus()
    await userEvent.keyboard("{ArrowDown}")
    await userEvent.click(
      await visible(await screen.findByRole("option", { name: /Opus/ }))
    )
    await expect(args.onChange).toHaveBeenCalledTimes(1)
    const alert = await screen.findByRole("alert")
    await expect(alert).toHaveTextContent(
      "Model was not changed: Opus is not available on this plan"
    )
    await waitFor(() => expect(model).toBeEnabled())
    await expect(model).toHaveTextContent("Sonnet")
    await settled()
  },
}

/** A question runs: nothing changes, and the reason reaches pointer and reader. */
export const DisabledDuringQuestion: Story = {
  args: {
    disabledReason: "Settings can be changed once the answer is done.",
  },
  play: async ({ canvasElement, args }) => {
    const screen = body()
    const model = screen.getByRole("combobox", { name: "Model: Sonnet" })
    await expect(model).toBeDisabled()
    const reasonId = model.getAttribute("aria-describedby")
    await expect(reasonId).not.toBeNull()
    await expect(
      canvasElement.ownerDocument.getElementById(reasonId as string)
    ).toHaveTextContent("Settings can be changed once the answer is done.")
    // A disabled button takes no pointer event: the wrapper carries the hover.
    await userEvent.hover(model.parentElement as HTMLElement)
    const [tip] = await screen.findAllByText(
      "Settings can be changed once the answer is done.",
      { selector: "[data-slot=tooltip-content], [data-slot=tooltip-content] *" }
    )
    await visible(tip as HTMLElement)
    await expect(args.onChange).not.toHaveBeenCalled()
  },
}

/** The narrowest column: the model stays, everything else goes behind « More ». */
export const Narrow240: Story = {
  args: { settings: CODEX_LIKE },
  decorators: [
    (Story) => (
      <div className="w-[240px] border p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement, args }) => {
    const screen = within(canvasElement)
    await expect(
      screen.getByRole("combobox", { name: "Model: GPT-5 Codex" })
    ).toBeVisible()
    await expect(
      screen.queryByRole("combobox", { name: /Reasoning effort/ })
    ).toBeNull()
    const more = screen.getByRole("button", { name: "More agent settings" })
    const row = canvasElement.querySelector('[role="group"]') as HTMLElement
    await expect(row.scrollWidth).toBeLessThanOrEqual(row.clientWidth)

    // Open with the keyboard, reach a setting, leave with Escape.
    more.focus()
    await userEvent.keyboard("{Enter}")
    await visible(
      await body().findByRole("menuitem", { name: /Reasoning effort/ })
    )
    await noIdIsShown(args.settings as AgentSettings)
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(body().queryByRole("menu")).toBeNull())
    await expect(args.onChange).not.toHaveBeenCalled()
    await settled()
  },
}

/** The agent's value is not in its own list: said as such, never as the id. */
export const UnknownCurrentValue: Story = {
  args: {
    settings: {
      modes: [choice("a", "Ask")],
      currentMode: "id-vanished-mode-7f3a",
      options: [
        select(
          "model",
          "Model",
          "model",
          [SONNET, OPUS],
          "id-retired-model-7f3a"
        ),
      ],
    },
  },
  play: async ({ args }) => {
    const screen = body()
    await expect(
      screen.getByRole("combobox", { name: "Model: Unlisted value" })
    ).toBeVisible()
    await expect(
      screen.getByRole("combobox", { name: "Mode: Unlisted value" })
    ).toBeVisible()
    await noIdIsShown(args.settings as AgentSettings)
  },
}

/** A state with a switch that stays inline at full width. */
const WITH_SWITCH: AgentSettings = {
  ...CLAUDE_LIKE,
  options: [
    ...CLAUDE_LIKE.options,
    toggle("autoaccept", "Auto-accept edits", "mode", false),
  ],
}

/**
 * Pinned to dark, so it survives a change of the Storybook default. Axe checks
 * the text; the boundary of an unticked switch it does not check at all.
 */
export const Dark: Story = {
  args: { settings: WITH_SWITCH },
  globals: { theme: "dark" },
}

/**
 * Storybook renders dark by default here, so light is the theme nothing would
 * otherwise check.
 */
export const Light: Story = {
  args: { settings: WITH_SWITCH },
  globals: { theme: "light" },
}
