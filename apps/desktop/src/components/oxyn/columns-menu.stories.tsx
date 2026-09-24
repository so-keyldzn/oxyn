import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ColumnsMenu } from "./columns-menu"
import { invoiceColumns } from "./fixtures"

const meta = {
  title: "Oxyn/ColumnsMenu",
  component: ColumnsMenu,
  args: {
    columns: invoiceColumns,
    hidden: new Set<number>(),
    onHiddenChange: fn(),
  },
  // The menu owns no state: the story holds it, as `ResultPanel` does.
  render: function Render(args) {
    const [hidden, setHidden] = React.useState(args.hidden)
    return (
      <div className="flex justify-end p-4">
        <ColumnsMenu
          {...args}
          hidden={hidden}
          onHiddenChange={(next) => {
            args.onHiddenChange(next)
            setHidden(next)
          }}
        />
      </div>
    )
  },
} satisfies Meta<typeof ColumnsMenu>

export default meta
type Story = StoryObj<typeof meta>

const body = within(document.body)

/** Hide one column, read the count, show them all again. */
export const HideAndShowAll: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Columns" }))
    await expect(
      await body.findByText("All 7 columns shown")
    ).toBeInTheDocument()
    // The export is not projected by what is hidden, and the menu says so.
    await expect(
      body.getByText(/An export keeps every column/)
    ).toBeInTheDocument()

    const customer = body.getByRole("menuitemcheckbox", { name: /customer/ })
    await expect(customer).toHaveAttribute("aria-checked", "true")
    await userEvent.click(customer)
    await expect(customer).toHaveAttribute("aria-checked", "false")
    await expect(body.getByText("6 of 7 columns shown")).toBeInTheDocument()
    // Hidden by its Arrow index: nothing renumbers.
    await expect(args.onHiddenChange).toHaveBeenLastCalledWith(new Set([1]))
    await expect(
      canvas.getByRole("button", { name: /Columns/ })
    ).toHaveTextContent("6/7")

    await userEvent.click(body.getByRole("menuitem", { name: "Show all" }))
    await expect(args.onHiddenChange).toHaveBeenLastCalledWith(new Set())
    await expect(body.getByText("All 7 columns shown")).toBeInTheDocument()
    await expect(customer).toHaveAttribute("aria-checked", "true")
    await expect(
      body.getByRole("menuitem", { name: "Show all" })
    ).toHaveAttribute("aria-disabled", "true")

    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
  },
}

/** The last shown column cannot be hidden: the grid never looks empty. */
export const LastShownColumn: Story = {
  args: { hidden: new Set([1, 2, 3, 4, 5, 6]) },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: /Columns/ }))
    await expect(
      await body.findByText("1 of 7 columns shown")
    ).toBeInTheDocument()
    await expect(
      body.getByRole("menuitemcheckbox", { name: /^id/ })
    ).toHaveAttribute("aria-disabled", "true")
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
  },
}
