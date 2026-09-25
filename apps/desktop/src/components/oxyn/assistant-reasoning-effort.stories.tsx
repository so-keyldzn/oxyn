import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantReasoningEffort } from "./assistant-reasoning-effort"

const body = () => within(document.body)

// Portalled popups animate in: wait for the frame where they are really there.
async function visible(element: HTMLElement) {
  await waitFor(() => expect(element).toBeVisible())
  return element
}

/** Every popup fully gone — Base UI's focus guards included — before axe runs. */
async function settled() {
  await waitFor(() =>
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  )
}

const meta = {
  title: "Oxyn/Assistant/ReasoningEffort",
  component: AssistantReasoningEffort,
  args: {
    efforts: ["low", "medium", "high", "xhigh", "max"],
    value: null,
    disabledReason: null,
    onChange: fn(),
  },
  decorators: [
    (Story) => (
      <div className="w-[560px] border p-2">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantReasoningEffort>

export default meta
type Story = StoryObj<typeof meta>

/** The model declares no level: nothing at all, not a greyed selector. */
export const NoneDeclared: Story = {
  args: { efforts: [] },
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelector('[data-slot="assistant-reasoning-effort"]')
    ).toBeNull()
    await expect(canvasElement.textContent).toBe("")
  },
}

/** Nothing chosen: « Default », never a level Oxyn guesses the provider uses. */
export const NothingChosen: Story = {
  play: async () => {
    await expect(
      body().getByRole("combobox", { name: "Reasoning effort: Default" })
    ).toBeVisible()
  },
}

/** Only the declared levels, in the provider's order, chosen by keyboard. */
export const ChooseByKeyboard: Story = {
  args: { efforts: ["low", "high", "max"], value: "low" },
  play: async ({ args }) => {
    const trigger = body().getByRole("combobox", {
      name: "Reasoning effort: Low",
    })
    trigger.focus()
    await userEvent.keyboard("{Enter}")
    const options = await body().findAllByRole("option")
    await visible(options[0] as HTMLElement)
    await expect(options.map((option) => option.textContent)).toEqual([
      "Low",
      "High",
      "Max",
    ])
    await userEvent.click(await body().findByRole("option", { name: "Max" }))
    await expect(args.onChange).toHaveBeenCalledWith("max")
    await settled()
  },
}

/**
 * The open list is named after its trigger, which Base UI does not do: this
 * story guards the dated exception in `components/ui/select.tsx` (front.md),
 * and fails if a shadcn regeneration erases it.
 */
export const ListIsNamedAfterItsTrigger: Story = {
  args: { value: "medium" },
  play: async () => {
    await userEvent.click(
      body().getByRole("combobox", { name: "Reasoning effort: Medium" })
    )
    await visible(
      await body().findByRole("listbox", {
        name: "Reasoning effort: Medium",
      })
    )
    await userEvent.keyboard("{Escape}")
    // Hidden once closed, but kept mounted: the accessibility tree, as axe
    // reads it, is what must be empty.
    await waitFor(() => expect(body().queryByRole("listbox")).toBeNull())
    await settled()
  },
}

/** A level the model no longer declares is shown as not chosen. */
export const ChosenForAnotherModel: Story = {
  args: { efforts: ["low", "high"], value: "max" },
  play: async () => {
    await expect(
      body().getByRole("combobox", { name: "Reasoning effort: Default" })
    ).toBeVisible()
  },
}

export const DisabledDuringQuestion: Story = {
  args: {
    value: "high",
    disabledReason: "The effort can be changed once the answer is done.",
  },
  play: async ({ canvasElement, args }) => {
    const trigger = body().getByRole("combobox", {
      name: "Reasoning effort: High",
    })
    await expect(trigger).toBeDisabled()
    const reasonId = trigger.getAttribute("aria-describedby")
    await expect(reasonId).not.toBeNull()
    await expect(
      canvasElement.ownerDocument.getElementById(reasonId as string)
    ).toHaveTextContent("The effort can be changed once the answer is done.")
    await expect(args.onChange).not.toHaveBeenCalled()
  },
}

/** The narrowest column: the selector fits without overflowing it. */
export const Narrow240: Story = {
  args: { value: "xhigh" },
  decorators: [
    (Story) => (
      <div className="w-[240px] border p-2" data-testid="column">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const column = within(canvasElement).getByTestId("column")
    await expect(
      body().getByRole("combobox", { name: "Reasoning effort: Extra high" })
    ).toBeVisible()
    await expect(column.scrollWidth).toBeLessThanOrEqual(column.clientWidth)
  },
}

/** Pinned to dark, so it survives a change of the Storybook default. */
export const Dark: Story = {
  args: { value: "medium" },
  globals: { theme: "dark" },
}

/** Storybook renders dark by default here: light is otherwise unchecked. */
export const Light: Story = {
  args: { value: "medium" },
  globals: { theme: "light" },
}
