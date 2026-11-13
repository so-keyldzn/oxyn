import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { OpenSessionTable } from "./open-session-table"

const meta = {
  title: "Oxyn/Open session table",
  component: OpenSessionTable,
  args: { onOpen: fn() },
} satisfies Meta<typeof OpenSessionTable>
export default meta
type Story = StoryObj<typeof meta>

export const NamedTable: Story = {
  play: async ({ canvasElement, args }) => {
    const page = within(canvasElement.ownerDocument.body)
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "Open table by name…" })
    )
    const dialog = await page.findByRole("dialog", {
      name: "Open a table on this session",
    })
    await expect(
      within(dialog).getByRole("button", { name: "Open table" })
    ).toBeDisabled()
    await userEvent.type(
      within(dialog).getByRole("textbox", { name: "Database" }),
      "demo"
    )
    await userEvent.type(
      within(dialog).getByRole("textbox", { name: "Table" }),
      "my`temporary table"
    )
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Open table" })
    )
    await expect(args.onOpen).toHaveBeenCalledWith("demo", "my`temporary table")
  },
}
