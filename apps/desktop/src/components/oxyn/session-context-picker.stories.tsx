import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { SessionContextPicker } from "./session-context-picker"

const choices = [
  { catalog: null, namespace: null },
  { catalog: "billing", namespace: "public" },
  { catalog: "billing", namespace: "analytics" },
]

const meta = {
  title: "Oxyn/SessionContextPicker",
  component: SessionContextPicker,
  decorators: [
    (Story) => (
      <div className="w-[520px] p-3">
        <Story />
      </div>
    ),
  ],
  args: {
    connectionName: "billing replica",
    choices,
    state: { status: "serverDefault" },
    onChoose: fn(),
    onCancel: fn(),
    onLoadSchemas: fn(),
  },
} satisfies Meta<typeof SessionContextPicker>

export default meta
type Story = StoryObj<typeof meta>

/** Choosing asks; the field does not move until the session answers. */
export const ServerDefault: Story = {
  play: async ({ canvas, args }) => {
    const select = canvas.getByLabelText("Session schema")
    await userEvent.selectOptions(select, "analytics")
    await expect(args.onChoose).toHaveBeenCalledWith({
      catalog: "billing",
      namespace: "analytics",
    })
    await expect(select).toHaveDisplayValue("Server default")
  },
}

export const Changing: Story = {
  args: {
    state: { status: "changing", target: "analytics", current: null },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: /Cancel context change/ })
    )
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

export const Declared: Story = {
  args: {
    state: {
      status: "declared",
      place: { catalog: "billing", namespace: "public" },
    },
  },
}

export const NoSchemaLoaded: Story = {
  args: { choices: [{ catalog: null, namespace: null }] },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Load schemas" }))
    await expect(args.onLoadSchemas).toHaveBeenCalled()
  },
}

/** A document store's namespaces are databases: the picker says so. */
export const DocumentDatabases: Story = {
  args: {
    level: "database",
    connectionName: "orders cluster",
    choices: [
      { catalog: null, namespace: null },
      { catalog: null, namespace: "shop" },
      { catalog: null, namespace: "audit" },
    ],
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.queryByLabelText("Session schema")).toBeNull()
    const select = canvas.getByLabelText("Session database")
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "does not claim to know the database."
    )
    await userEvent.selectOptions(select, "shop")
    await expect(args.onChoose).toHaveBeenCalledWith({
      catalog: null,
      namespace: "shop",
    })
  },
}

/**
 * A Redis session: its databases hold no schema, so they are the places —
 * named as the tree names them, and no « Load schemas » to chase.
 */
export const DatabasesOfAKeyValueStore: Story = {
  args: {
    connectionName: "cache",
    choices: [
      { catalog: null, namespace: null },
      { catalog: "db0", namespace: null },
      { catalog: "db3", namespace: null },
    ],
    state: { status: "declared", place: { catalog: "db0", namespace: null } },
  },
  play: async ({ canvas, args }) => {
    const select = canvas.getByLabelText("Session schema")
    await expect(select).toHaveDisplayValue("db0")
    await expect(
      canvas.queryByRole("button", { name: "Load schemas" })
    ).not.toBeInTheDocument()
    await userEvent.selectOptions(select, "db3")
    await expect(args.onChoose).toHaveBeenCalledWith({
      catalog: "db3",
      namespace: null,
    })
  },
}

/** No database read yet: loading them is offered in the same words. */
export const NoDatabaseLoaded: Story = {
  args: { level: "database", choices: [{ catalog: null, namespace: null }] },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "The catalog lists no database for this connection yet."
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Load databases" })
    )
    await expect(args.onLoadSchemas).toHaveBeenCalled()
  },
}

export const Failed: Story = {
  args: {
    state: {
      status: "failed",
      message: 'ERROR: schema "analytics" does not exist (3F000)',
      retryable: false,
      current: { catalog: "billing", namespace: "public" },
    },
  },
}
