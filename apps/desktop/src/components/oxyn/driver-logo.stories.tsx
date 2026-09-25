import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { DriverLogo } from "./driver-logo"

const meta = {
  title: "Oxyn/DriverLogo",
  component: DriverLogo,
  args: { driver: "postgres", className: "size-8" },
} satisfies Meta<typeof DriverLogo>

export default meta
type Story = StoryObj<typeof meta>

function logo(canvasElement: HTMLElement) {
  const svg = canvasElement.querySelector("svg")
  if (svg === null) throw new Error("no logo drawn")
  return svg
}

/**
 * A known driver wears its official mark, hidden from assistive technology:
 * the driver's name is always written next to it.
 */
export const Postgres: Story = {
  play: async ({ canvasElement }) => {
    const svg = logo(canvasElement)
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    await expect(svg).toHaveClass("size-8")
    await expect(svg).toBeVisible()
  },
}

/** SQLite's mark is framed out of the horizontal lockup: square, no wordmark. */
export const Sqlite: Story = {
  args: { driver: "sqlite" },
  play: async ({ canvasElement }) => {
    const svg = logo(canvasElement)
    await expect(svg).toHaveAttribute("viewBox", "0 0 206 228")
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    await expect(svg.querySelector("clipPath")).not.toBeNull()
  },
}

/** A driver without a known mark is never drawn without one. */
export const UnknownDriver: Story = {
  args: { driver: "a-driver-without-a-mark" },
  play: async ({ canvasElement }) => {
    const svg = logo(canvasElement)
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    await expect(svg).toBeVisible()
  },
}

/** Both marks stay legible on the dark theme. */
export const Dark: Story = {
  args: { driver: "sqlite" },
  decorators: [
    (Story) => (
      <div className="dark flex gap-3 bg-background p-3 text-foreground">
        <Story />
        <DriverLogo driver="postgres" className="size-8" />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelectorAll("svg[aria-hidden]")
    ).toHaveLength(2)
  },
}
