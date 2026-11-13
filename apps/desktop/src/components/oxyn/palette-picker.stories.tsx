import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { PalettePicker } from "./palette-picker"

const meta = {
  title: "Oxyn/Settings/PalettePicker",
  component: PalettePicker,
  args: { mode: "light", value: "oxyn", onChange: fn() },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PalettePicker>
export default meta
type Story = StoryObj<typeof meta>

export const LightPalette: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Ocean" }))
    await expect(args.onChange).toHaveBeenCalledWith("ocean")
  },
}
export const DarkPalette: Story = { args: { mode: "dark", value: "plum" } }
export const Unavailable: Story = {
  args: { disabled: true },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("button", { name: "Ocean" })).toBeDisabled()
  },
}
