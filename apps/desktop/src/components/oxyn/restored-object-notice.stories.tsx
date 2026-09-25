import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { HOSTILE } from "./metadata-fixtures"
import { RestoredObjectNotice } from "./restored-object-notice"

const meta = {
  title: "Oxyn/RestoredObjectNotice",
  component: RestoredObjectNotice,
  decorators: [
    (Story) => (
      <div className="w-[640px] border">
        <Story />
      </div>
    ),
  ],
  args: { state: "held", name: "invoices", onRead: fn() },
} satisfies Meta<typeof RestoredObjectNotice>

export default meta
type Story = StoryObj<typeof meta>

/** Back from the last session: nothing read until the user asks. */
export const Held: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("Nothing has been read from the server yet.")
    ).toBeVisible()
    await expect(args.onRead).not.toHaveBeenCalled()
    await userEvent.click(canvas.getByRole("button", { name: "Read it now" }))
    await expect(args.onRead).toHaveBeenCalledOnce()
  },
}

/** Read again and gone: explained, its place kept, nothing offered to erase. */
export const Vanished: Story = {
  args: { state: "vanished", name: HOSTILE, onRead: undefined },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/was not found/)).toBeVisible()
    await expect(canvas.getByText(HOSTILE).querySelector("*")).toBeNull()
    await expect(canvas.getByText(/Its place is kept/)).toBeVisible()
    await expect(canvas.queryByRole("button")).toBeNull()
  },
}

export const Light: Story = { globals: { theme: "light" } }
