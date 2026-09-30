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

export const MySQL: Story = {
  args: { driver: "mysql" },
  globals: { theme: "light" },
  play: async ({ canvasElement }) => {
    const marks = canvasElement.querySelectorAll('svg[viewBox="0 0 256 252"]')
    await expect(marks).toHaveLength(1)
    const svg = logo(canvasElement)
    await expect(svg).toBeVisible()
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    // axe does not measure decorative SVG fills; check both paths in-browser.
    for (const path of svg.querySelectorAll("path"))
      await expect(getComputedStyle(path).fill).toBe("rgb(0, 84, 107)")
  },
}

export const MySQLDark: Story = {
  args: { driver: "mysql" },
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => {
    const marks = canvasElement.querySelectorAll('svg[viewBox="0 0 256 252"]')
    await expect(marks).toHaveLength(1)
    const svg = logo(canvasElement)
    await expect(svg).toBeVisible()
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    for (const path of svg.querySelectorAll("path"))
      await expect(getComputedStyle(path).fill).toBe("rgb(255, 255, 255)")
  },
}
