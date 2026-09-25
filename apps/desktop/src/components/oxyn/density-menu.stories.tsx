import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { DensityMenu } from "./density-menu"

const meta = {
  title: "Oxyn/DensityMenu",
  component: DensityMenu,
  args: {
    value: "compact",
    onChange: fn(),
  },
} satisfies Meta<typeof DensityMenu>

export default meta
type Story = StoryObj<typeof meta>

/**
 * Axe runs after `play`, on whatever is still mounted: a menu animating out
 * keeps its focus guards and its items for a few frames, and the suite's pace
 * decides whether axe sees them.
 */
async function closeMenu() {
  await userEvent.keyboard("{Escape}")
  await waitFor(() => {
    expect(within(document.body).queryByRole("menu")).toBeNull()
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  })
}

export const Compact: Story = {
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", { name: "Text size: Compact" })
    ).toHaveTextContent("Compact")
  },
}

export const Comfortable: Story = {
  args: { value: "comfortable" },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByRole("button", {
        name: "Text size: Comfortable",
      })
    ).toHaveTextContent("Comfortable")
  },
}

/** The choice reaches the preference as its value, not as its label. */
export const ChoosingASize: Story = {
  play: async ({ args, canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "Text size: Compact" })
    )
    const body = within(document.body)
    await expect(
      await body.findByRole("menuitemradio", { name: /Compact/ })
    ).toHaveAttribute("aria-checked", "true")
    await userEvent.click(
      body.getByRole("menuitemradio", { name: /Comfortable/ })
    )
    await expect(args.onChange).toHaveBeenCalledOnce()
    await expect(args.onChange).toHaveBeenCalledWith("comfortable")
    await closeMenu()
  },
}
