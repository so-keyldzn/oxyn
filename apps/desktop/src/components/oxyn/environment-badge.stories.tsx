import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { EnvironmentBadge } from "./environment-badge"
import { ENVIRONMENTS } from "@/lib/ipc/types"

const meta = {
  title: "Oxyn/EnvironmentBadge",
  component: EnvironmentBadge,
  parameters: { layout: "centered" },
  args: { environment: "production" },
} satisfies Meta<typeof EnvironmentBadge>

export default meta
type Story = StoryObj<typeof meta>

export const Production: Story = {
  play: async ({ canvas }) => {
    // The marking is written out: colour alone never carries it.
    const badge = canvas.getByText("PRODUCTION")
    await expect(badge).toBeVisible()
    // Built on `Badge`, so the pill and its slot come from one place…
    await expect(badge).toHaveAttribute("data-slot", "environment-badge")
    // …and the caption size still wins over the badge's own `text-xs`: the
    // bare `text-caption` form is dropped by `cn` as if it were a colour.
    await expect(getComputedStyle(badge).fontSize).toBe("11px")
  },
}

/** Props reach the element, so a caller can describe or address it. */
export const PassesProps: Story = {
  args: { environment: "staging", "aria-describedby": "why", id: "env" },
  play: async ({ canvas }) => {
    const badge = canvas.getByText("Staging")
    await expect(badge).toHaveAttribute("id", "env")
    await expect(badge).toHaveAttribute("aria-describedby", "why")
    await expect(badge).toHaveAttribute("data-environment", "staging")
  },
}

export const AllEnvironments: Story = {
  render: () => (
    <div className="flex gap-3 p-6">
      {ENVIRONMENTS.map((environment) => (
        <EnvironmentBadge key={environment} environment={environment} />
      ))}
    </div>
  ),
}

export const AllEnvironmentsLight: Story = {
  ...AllEnvironments,
  globals: { theme: "light" },
}
