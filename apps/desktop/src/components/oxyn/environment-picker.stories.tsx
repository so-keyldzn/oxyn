import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor } from "storybook/test"

import { EnvironmentPicker } from "./environment-picker"
import type { Environment } from "@/lib/ipc/types"

function Controlled({
  initial,
  disabled,
}: {
  initial: Environment
  disabled?: boolean
}) {
  const [value, setValue] = React.useState(initial)
  return (
    <EnvironmentPicker value={value} onChange={setValue} disabled={disabled} />
  )
}

const meta = {
  title: "Oxyn/EnvironmentPicker",
  component: Controlled,
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-6">
        <Story />
      </div>
    ),
  ],
  args: { initial: "production" },
} satisfies Meta<typeof Controlled>

export default meta
type Story = StoryObj<typeof meta>

export const ProductionByDefault: Story = {
  play: async ({ canvas }) => {
    // An exclusive choice: radios, the default checked (I-02).
    const production = canvas.getByRole("radio", { name: /PRODUCTION/ })
    await expect(production).toBeChecked()
    await expect(canvas.getAllByRole("radio")).toHaveLength(4)
  },
}

export const KeyboardOnly: Story = {
  play: async ({ canvas }) => {
    canvas.getByRole("radio", { name: /PRODUCTION/ }).focus()
    await userEvent.keyboard("{ArrowRight}")
    await waitFor(() =>
      expect(canvas.getByRole("radio", { name: /Staging/ })).toBeChecked()
    )
    await expect(
      canvas.getByRole("radio", { name: /PRODUCTION/ })
    ).not.toBeChecked()
  },
}

export const LocalChosen: Story = { args: { initial: "local" } }

export const Disabled: Story = { args: { initial: "staging", disabled: true } }

export const Light: Story = { globals: { theme: "light" } }

export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="w-[360px]">
        <Story />
      </div>
    ),
  ],
}
