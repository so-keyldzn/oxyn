import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { SqlEditor } from "./sql-editor"
import { matchesSummary } from "./sql-editor-search"
import { modKey } from "@/lib/actions/platform"

// ⌘ on macOS, Ctrl elsewhere: the registry's dispatcher, installed by the
// preview as in the window, reads it from the platform.
const mod = (keys: string) => `{${modKey}>}${keys}{/${modKey}}`

const counted = (count: number) =>
  matchesSummary({ state: "counted", count, capped: false })

// The find bar is part of the SQL editor: it is opened in one, by ⌘F, and acts
// on its text.
const meta = {
  title: "Oxyn/SqlEditorSearch",
  component: SqlEditor,
  args: {
    value:
      "-- Invoices unpaid for more than 30 days\nSELECT c.name, i.amount, i.issued_at\nFROM invoices AS i\nJOIN customers AS c ON c.id = i.customer_id\nWHERE i.paid_at IS NULL\n  AND i.issued_at < now() - interval '30 days'\nORDER BY i.issued_at;",
    driver: "postgres",
    onChange: fn(),
  },
  render: function Render(args) {
    const [value, setValue] = React.useState(args.value)
    return (
      <div className="h-[320px] border">
        <SqlEditor
          {...args}
          value={value}
          onChange={(next) => {
            setValue(next)
            args.onChange(next)
          }}
        />
      </div>
    )
  },
} satisfies Meta<typeof SqlEditor>

export default meta
type Story = StoryObj<typeof meta>

/** ⌘F searches the text of the editor that has the focus. */
export const CommandFFindsInTheEditor: Story = {
  play: async ({ canvas, canvasElement }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard(mod("f"))
    await waitFor(() =>
      expect(canvasElement.querySelector('[role="search"]')).not.toBeNull()
    )
    // The field takes the focus: the user types what to find.
    await expect(
      await canvas.findByRole("textbox", { name: "Find" })
    ).toHaveFocus()
  },
}

/**
 * The find bar is drawn with the interface's components, over CodeMirror's own
 * search: the count follows what is typed, the options refine it, and replacing
 * changes the text — which is what `onChange` reports.
 */
export const FindAndReplace: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("textbox", { name: "SQL editor" }))
    await userEvent.keyboard(mod("f"))
    const find = await canvas.findByRole("textbox", { name: "Find" })
    await userEvent.type(find, "invoices")
    // « Invoices » in the comment and `invoices` in the query: case is ignored.
    await waitFor(() =>
      expect(canvas.getByRole("status")).toHaveTextContent(counted(2))
    )
    await userEvent.click(canvas.getByRole("button", { name: "Match case" }))
    await waitFor(() =>
      expect(canvas.getByRole("status")).toHaveTextContent(counted(1))
    )
    await userEvent.type(
      canvas.getByRole("textbox", { name: "Replace" }),
      "bills"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Replace all" }))
    await waitFor(() =>
      expect(args.onChange).toHaveBeenLastCalledWith(
        expect.stringContaining("FROM bills AS i")
      )
    )
    // The match that differs by case was not touched.
    await expect(args.onChange).toHaveBeenLastCalledWith(
      expect.stringContaining("-- Invoices unpaid")
    )
  },
}

/** A regular expression that does not parse is said, not silently ignored. */
export const AnInvalidRegularExpressionIsSaid: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("textbox", { name: "SQL editor" }))
    await userEvent.keyboard(mod("f"))
    const find = await canvas.findByRole("textbox", { name: "Find" })
    await userEvent.click(
      canvas.getByRole("button", { name: "Regular expression" })
    )
    await userEvent.type(find, "(")
    await waitFor(() =>
      expect(canvas.getByRole("status")).toHaveTextContent(
        matchesSummary({ state: "invalid" })
      )
    )
    await expect(find).toHaveAttribute("aria-invalid", "true")
  },
}

/** A read-only view can be searched, and offers no replacing. */
export const ReadOnlyFindsButDoesNotReplace: Story = {
  args: { readOnly: true },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard(mod("f"))
    await canvas.findByRole("textbox", { name: "Find" })
    await expect(canvas.queryByRole("textbox", { name: "Replace" })).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: "Replace all" })
    ).toBeNull()
  },
}
