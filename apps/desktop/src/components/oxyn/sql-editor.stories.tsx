import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { SqlEditor } from "./sql-editor"

const meta = {
  title: "Oxyn/SqlEditor",
  component: SqlEditor,
  args: {
    value:
      "-- Invoices unpaid for more than 30 days\nSELECT c.name, i.amount, i.issued_at\nFROM invoices AS i\nJOIN customers AS c ON c.id = i.customer_id\nWHERE i.paid_at IS NULL\n  AND i.issued_at < now() - interval '30 days'\nORDER BY i.issued_at;",
    driver: "postgres",
    onChange: fn(),
    onRun: fn(),
    onRunAll: fn(),
    onSave: fn(),
    onCancel: fn(),
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

export const PostgreSQL: Story = {
  play: async ({ canvas, args }) => {
    const editor = canvas.getByRole("textbox")
    await userEvent.click(editor)
    await userEvent.keyboard("{Meta>}{Enter}{/Meta}")
    await waitFor(() => expect(args.onRun).toHaveBeenCalledTimes(1))
    // No selection: the statement under the cursor is the target.
    await expect(args.onRun).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "statement" })
    )
  },
}

export const RunningEscapeCancels: Story = {
  args: { running: true },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(args.onCancel).toHaveBeenCalled())
    // While running, ⌘↵ does not start a second statement.
    await userEvent.keyboard("{Meta>}{Enter}{/Meta}")
    await expect(args.onRun).not.toHaveBeenCalled()
  },
}

export const SelectionAndShortcuts: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("textbox"))
    await userEvent.keyboard("{Meta>}a{/Meta}")
    await userEvent.keyboard("{Meta>}{Enter}{/Meta}")
    await waitFor(() =>
      expect(args.onRun).toHaveBeenCalledWith(
        expect.objectContaining({ kind: "selection", start: 0 })
      )
    )
    await userEvent.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}")
    await waitFor(() => expect(args.onRunAll).toHaveBeenCalledTimes(1))
    await userEvent.keyboard("{Meta>}s{/Meta}")
    await waitFor(() => expect(args.onSave).toHaveBeenCalledTimes(1))
  },
}

/**
 * A read-only view of a stored query: selection and copy work, and no
 * shortcut runs or saves it (docs/UX-SPEC.md, « Consultation locale des
 * requêtes »). ⌘↵ there would execute a text the user only meant to read.
 */
export const ReadOnlyRunsNothing: Story = {
  args: { readOnly: true },
  play: async ({ canvas, args }) => {
    const editor = canvas.getByRole("textbox")
    await userEvent.click(editor)
    await userEvent.keyboard("{Meta>}a{/Meta}")
    await userEvent.keyboard("{Meta>}{Enter}{/Meta}")
    await userEvent.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}")
    await userEvent.keyboard("{Meta>}s{/Meta}")
    await expect(args.onRun).not.toHaveBeenCalled()
    await expect(args.onRunAll).not.toHaveBeenCalled()
    await expect(args.onSave).not.toHaveBeenCalled()
  },
}

export const SQLite: Story = {
  args: {
    driver: "sqlite",
    value: "SELECT name FROM sqlite_master WHERE type = 'table';",
  },
}
