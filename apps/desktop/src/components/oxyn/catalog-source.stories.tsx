import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, within } from "storybook/test"

import { CatalogSource } from "./catalog-source"
import { chooseSelectOption } from "./select-story"

const meta = {
  title: "Oxyn/Catalog source",
  component: CatalogSource,
  args: {
    value: "",
    choices: [
      { id: "first", label: "Console 1" },
      { id: "second", label: "Console 2" },
    ],
    onChange: fn(),
  },
} satisfies Meta<typeof CatalogSource>
export default meta
type Story = StoryObj<typeof meta>

export const Connection: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await chooseSelectOption(
      canvas.getByRole("combobox", { name: "Browse session" }),
      "Console 2"
    )
    await expect(args.onChange).toHaveBeenCalledWith("second")
  },
}
export const Console: Story = {
  args: { value: "first" },
  play: async ({ canvasElement, args }) => {
    await chooseSelectOption(
      within(canvasElement).getByRole("combobox", { name: "Browse session" }),
      "Connection catalog"
    )
    await expect(args.onChange).toHaveBeenCalledWith("")
  },
}
export const NoOpenConsole: Story = { args: { choices: [] } }
