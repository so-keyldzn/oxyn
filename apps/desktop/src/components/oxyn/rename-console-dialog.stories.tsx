import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { RenameConsoleDialog } from "./rename-console-dialog"

const meta = {
  title: "Oxyn/RenameConsoleDialog",
  component: RenameConsoleDialog,
  args: {
    title: "unpaid invoices.sql",
    onCancel: fn(),
    onRename: fn(),
  },
} satisfies Meta<typeof RenameConsoleDialog>

export default meta
type Story = StoryObj<typeof meta>

/** The field starts from the console's name; Enter renames. */
export const Renames: Story = {
  play: async ({ args }) => {
    const page = within(document.body)
    const field = await page.findByRole("textbox", { name: "Query name" })
    await expect(field).toHaveValue("unpaid invoices.sql")
    // No correction nor typographic quote in a name (ADR-0041 § 8).
    await expect(field).toHaveAttribute("spellcheck", "false")
    await userEvent.clear(field)
    await userEvent.type(field, "overdue.sql{Enter}")
    await expect(args.onRename).toHaveBeenCalledWith("overdue.sql")
  },
}

/** An empty name is refused with its reason, as the save would refuse it. */
export const EmptyNameIsRefused: Story = {
  play: async ({ args }) => {
    const page = within(document.body)
    const field = await page.findByRole("textbox", { name: "Query name" })
    await userEvent.clear(field)
    await waitFor(() =>
      expect(page.getByText("A query needs a name.")).toBeVisible()
    )
    await expect(page.getByRole("button", { name: "Rename" })).toBeDisabled()
    await userEvent.keyboard("{Enter}")
    await expect(args.onRename).not.toHaveBeenCalled()
    await userEvent.click(page.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(args.onCancel).toHaveBeenCalled())
  },
}

export const HostileName: Story = {
  args: { title: '<img src=x onerror=alert(1)>"; DROP TABLE audit; --' },
  play: async () => {
    const page = within(document.body)
    const heading = await page.findByRole("heading", {
      name: /DROP TABLE audit/,
    })
    await waitFor(() => expect(heading).toBeVisible())
    await expect(page.queryByRole("img", { name: "x" })).toBeNull()
  },
}
