import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { cn } from "@/lib/utils"

import { DriverLogo } from "./driver-logo"
import { catalogueDrivers } from "./fixtures"

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

/** The leaf in svgl's light-surface slate, one path, no wordmark. */
export const MongoDB: Story = {
  args: { driver: "mongodb" },
  globals: { theme: "light" },
  play: async ({ canvasElement }) => {
    const svg = logo(canvasElement)
    await expect(svg).toHaveAttribute("viewBox", "0 0 120 258")
    await expect(svg).toBeVisible()
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    const paths = svg.querySelectorAll("path")
    await expect(paths).toHaveLength(1)
    for (const path of paths)
      await expect(getComputedStyle(path).fill).toBe("rgb(0, 30, 43)")
  },
}

/** On a dark surface the slate would vanish: the leaf takes svgl's green. */
export const MongoDBDark: Story = {
  args: { driver: "mongodb" },
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => {
    const svg = logo(canvasElement)
    await expect(svg).toBeVisible()
    await expect(svg).toHaveAttribute("aria-hidden", "true")
    for (const path of svg.querySelectorAll("path"))
      await expect(getComputedStyle(path).fill).toBe("rgb(0, 237, 100)")
  },
}

export const RedisMark: Story = {
  args: { driver: "redis" },
  globals: { theme: "light" },
  play: async ({ canvasElement }) => {
    const marks = canvasElement.querySelectorAll('svg[viewBox="0 0 256 220"]')
    await expect(marks).toHaveLength(1)
    const svg = logo(canvasElement)
    await expect(svg).toBeVisible()
    await expect(svg).toHaveAttribute("aria-hidden", "true")
  },
}

/** svgl's Redis mark is red on both themes: no token override is needed. */
export const RedisMarkDark: Story = {
  args: { driver: "redis" },
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelectorAll('svg[viewBox="0 0 256 220"]')
    ).toHaveLength(1)
    await expect(logo(canvasElement)).toBeVisible()
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

const markSets = [
  { theme: "light", size: "size-4" },
  { theme: "light", size: "size-5" },
  { theme: "dark", size: "size-4" },
  { theme: "dark", size: "size-5" },
] as const

function drawn(set: Element, driver: string) {
  const svg = set.querySelector(`[data-driver="${driver}"] svg`)
  if (svg === null) throw new Error(`no logo drawn for ${driver}`)
  return svg
}

/**
 * Every type of the catalogue, at the sizes the interface draws them, on both
 * themes. A driver added to the catalogue without a mark fails here; Oracle
 * alone keeps the generic icon, on purpose. Each set holds every mark once:
 * two marks sharing an `id` would let one's `url(#…)` resolve to the other's
 * gradient or clip, drawing the wrong shape.
 */
export const EveryCatalogueDriver: Story = {
  globals: { theme: "light" },
  render: () => (
    <div className="flex flex-col gap-3 p-3">
      <div hidden data-generic>
        <DriverLogo driver="a-driver-without-a-mark" className="size-4" />
      </div>
      {markSets.map(({ theme, size }) => (
        <ul
          key={`${theme}-${size}`}
          aria-label={`${theme} theme, ${size}`}
          data-mark-set
          className={cn(
            theme === "dark" && "dark",
            "grid grid-cols-6 gap-2 rounded-md bg-background p-3 text-xs text-foreground"
          )}
        >
          {catalogueDrivers.map(({ id, displayName }) => (
            <li key={id} data-driver={id} className="flex items-center gap-1.5">
              <DriverLogo driver={id} className={size} />
              {displayName}
            </li>
          ))}
        </ul>
      ))}
    </div>
  ),
  play: async ({ canvasElement }) => {
    const generic = canvasElement.querySelector("[data-generic] svg")
    if (generic === null) throw new Error("no generic icon drawn")
    const sets = canvasElement.querySelectorAll("[data-mark-set]")
    await expect(sets).toHaveLength(markSets.length)
    for (const set of sets) {
      for (const { id } of catalogueDrivers) {
        const svg = drawn(set, id)
        await expect(svg).toHaveAttribute("aria-hidden", "true")
        await expect(svg).toBeVisible()
        if (id === "oracle") await expect(svg.innerHTML).toBe(generic.innerHTML)
        else
          await expect(svg.innerHTML, `${id} wears a mark`).not.toBe(
            generic.innerHTML
          )
      }
      const ids = Array.from(set.querySelectorAll("[id]"), (e) => e.id)
      await expect(ids.filter((id, i) => ids.indexOf(id) !== i)).toEqual([])
    }
  },
}

async function wornMark(canvasElement: HTMLElement, viewBox: string) {
  const svg = logo(canvasElement)
  await expect(svg).toHaveAttribute("aria-hidden", "true")
  await expect(svg).toHaveAttribute("viewBox", viewBox)
  await expect(svg).toBeVisible()
  return svg
}

/** ClickHouse's own square: yellow ground, legible on either theme. */
export const ClickHouse: Story = {
  args: { driver: "clickhouse" },
  globals: { theme: "light" },
  play: async ({ canvasElement }) => {
    await wornMark(canvasElement, "0 0 616 616")
  },
}

export const ClickHouseDark: Story = {
  args: { driver: "clickhouse" },
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => {
    await wornMark(canvasElement, "0 0 616 616")
  },
}

/** DuckDB's disc, framed out of the lockup the vendor drew for a light page. */
export const DuckDb: Story = {
  args: { driver: "duckdb" },
  globals: { theme: "light" },
  play: async ({ canvasElement }) => {
    const svg = await wornMark(canvasElement, "0 0 334.4 334.4")
    await expect(
      svg.querySelector('svg[viewBox="0 0 769.5 591.9"]')
    ).toBeVisible()
    await expect(
      svg.querySelector('svg[viewBox="0 0 802.9 609.4"]')
    ).not.toBeVisible()
  },
}

/** …and out of the yellow lockup the vendor drew for a dark page. */
export const DuckDbDark: Story = {
  args: { driver: "duckdb" },
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => {
    const svg = await wornMark(canvasElement, "0 0 334.4 334.4")
    await expect(
      svg.querySelector('svg[viewBox="0 0 802.9 609.4"]')
    ).toBeVisible()
    await expect(
      svg.querySelector('svg[viewBox="0 0 769.5 591.9"]')
    ).not.toBeVisible()
  },
}

/** Elasticsearch's middle bar takes the text color, as in Elastic's own UI. */
export const Elasticsearch: Story = {
  args: { driver: "elasticsearch" },
  globals: { theme: "light" },
  play: async ({ canvasElement }) => {
    const svg = await wornMark(canvasElement, "0 0 32 32")
    const bar = svg.querySelector(".euiIcon__fillNegative")
    if (bar === null) throw new Error("no middle bar")
    await expect(getComputedStyle(bar).fill).not.toBe("none")
  },
}

export const ElasticsearchDark: Story = {
  args: { driver: "elasticsearch" },
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => {
    const svg = await wornMark(canvasElement, "0 0 32 32")
    const bar = svg.querySelector(".euiIcon__fillNegative")
    if (bar === null) throw new Error("no middle bar")
    // Left to the SVG default, the bar would be black on a dark surface.
    await expect(getComputedStyle(bar).fill).not.toBe("rgb(0, 0, 0)")
    await expect(getComputedStyle(bar).fill).not.toBe("none")
  },
}
