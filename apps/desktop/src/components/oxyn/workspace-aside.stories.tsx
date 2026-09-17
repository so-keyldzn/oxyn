import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"
import { InformationCircleIcon, SparklesIcon } from "@hugeicons/core-free-icons"

import { WorkspaceAside } from "./workspace-aside"

const items = [
  {
    id: "inspector",
    label: "Inspector",
    icon: InformationCircleIcon,
    content: <p className="p-3 text-sm">Row 12 of invoices</p>,
  },
  {
    id: "assistant",
    label: "Assistant",
    icon: SparklesIcon,
    content: <textarea aria-label="Ask the assistant" className="m-3 border" />,
  },
]

const meta = {
  title: "Oxyn/WorkspaceAside",
  component: WorkspaceAside,
  decorators: [
    (Story) => (
      <div className="h-[420px] w-[320px] border">
        <Story />
      </div>
    ),
  ],
  args: { items, active: "inspector", onActiveChange: fn(), onClose: fn() },
  render: function Render(args) {
    const [active, setActive] = React.useState(args.active)
    return (
      <WorkspaceAside
        {...args}
        active={active}
        onActiveChange={(id) => {
          setActive(id)
          args.onActiveChange(id)
        }}
      />
    )
  },
} satisfies Meta<typeof WorkspaceAside>

export default meta
type Story = StoryObj<typeof meta>

/** A hidden panel keeps what was typed in it. */
export const TwoPanels: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("tab", { name: /Assistant/ }))
    await userEvent.type(canvas.getByLabelText("Ask the assistant"), "why")
    await userEvent.click(canvas.getByRole("tab", { name: /Inspector/ }))
    await userEvent.click(canvas.getByRole("tab", { name: /Assistant/ }))
    await expect(canvas.getByLabelText("Ask the assistant")).toHaveValue("why")
  },
}

export const OnePanel: Story = {
  args: { items: items.slice(0, 1) },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: /Close side panel/ })
    )
    await expect(args.onClose).toHaveBeenCalled()
  },
}

/** The inactive tab on `--sidebar` in light: 4.4:1 before, checked by axe here. */
export const TwoPanelsLight: Story = {
  ...TwoPanels,
  play: undefined,
  globals: { theme: "light" },
}
