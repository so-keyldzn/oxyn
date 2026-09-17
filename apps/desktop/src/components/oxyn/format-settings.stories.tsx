import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { FormatSettings } from "./format-settings"
import { DEFAULT_PREFERENCES } from "@/features/settings/preferences"

const meta = {
  title: "Oxyn/FormatSettings",
  component: FormatSettings,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: { preferences: DEFAULT_PREFERENCES, onChange: fn() },
} satisfies Meta<typeof FormatSettings>

export default meta
type Story = StoryObj<typeof meta>

export const Defaults: Story = {
  play: async ({ canvas, args }) => {
    await expect(canvas.getByTestId("grouping-preview")).toHaveTextContent(
      "4823917"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Thousands" }))
    await expect(args.onChange).toHaveBeenCalledWith({ groupThousands: true })
    await userEvent.click(canvas.getByRole("button", { name: "Size only" }))
    await expect(args.onChange).toHaveBeenCalledWith({ binaryDisplay: "size" })
    await userEvent.selectOptions(canvas.getByLabelText("Cell length"), "2048")
    await expect(args.onChange).toHaveBeenCalledWith({ cellMaxChars: 2048 })
  },
}

export const GroupedWithSizes: Story = {
  args: {
    preferences: {
      ...DEFAULT_PREFERENCES,
      groupThousands: true,
      binaryDisplay: "size",
      cellMaxChars: 8192,
    },
  },
}

export const MarkerTooLong: Story = {
  play: async ({ canvas, args }) => {
    const input = canvas.getByLabelText("Missing value marker")
    await userEvent.clear(input)
    await userEvent.type(input, "x".repeat(65))
    await waitFor(() =>
      expect(canvas.getByText("At most 64 bytes.")).toBeVisible()
    )
    // The last change sent stays inside the bound the domain enforces.
    await expect(args.onChange).not.toHaveBeenCalledWith({
      nullText: "x".repeat(65),
    })
  },
}
