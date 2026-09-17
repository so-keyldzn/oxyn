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
