import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { CatalogSource } from "./catalog-source"

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
    await userEvent.selectOptions(
      canvas.getByRole("combobox", { name: "Browse session" }),
      "second"
    )
    await expect(args.onChange).toHaveBeenCalledWith("second")
  },
}
export const Console: Story = { args: { value: "first" } }
export const NoOpenConsole: Story = { args: { choices: [] } }
