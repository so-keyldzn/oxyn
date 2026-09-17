import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantSources } from "./assistant-sources"
import type { AssistantSource } from "./assistant-sources"

const SOURCES: Array<AssistantSource> = [
  {
    key: "s1",
    address: { catalog: "shop", namespace: "public", relation: "invoices" },
    facet: "Structure",
    rows: null,
  },
  {
    key: "s2",
    address: { catalog: "shop", namespace: "public", relation: "customers" },
    facet: "Data",
    rows: 200,
  },
  {
    key: "s3",
    address: { catalog: "shop", namespace: "public", relation: "order_lines" },
    facet: "DDL",
    rows: null,
  },
]

const meta = {
  title: "Oxyn/Assistant/Sources",
  component: AssistantSources,
  args: {
    sources: SOURCES,
    tier: "metadata",
    defaultOpen: true,
    onOpenObject: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantSources>

export default meta
type Story = StoryObj<typeof meta>

export const UnderMetadata: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Oxyn read 3 objects")).toBeVisible()
    // Under `metadata` no row value left the machine, and the count says so
    // rather than letting the reader assume it did (ADR-0006).
    await expect(
      canvas.getByText("200 rows read · their values stayed on this machine")
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Open shop.public.customers" })
    )
    await expect(args.onOpenObject).toHaveBeenCalledWith({
      catalog: "shop",
      namespace: "public",
      relation: "customers",
    })
  },
}

/** The only tier under which values were sent, said as such. */
export const UnderSampled: Story = {
  args: { tier: "sampled" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText("200 rows read · a sample you approved was sent")
    ).toBeVisible()
  },
}

export const UnderLocal: Story = {
  args: { tier: "local" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText("200 rows read · their values stayed on this machine")
    ).toBeVisible()
  },
}

/** Read, then dropped: the row stays, the action does not. */
export const ObjectGoneFromTheCatalog: Story = {
  args: {
    sources: [
      SOURCES[0]!,
      {
        key: "s9",
        address: {
          catalog: "shop",
          namespace: "public",
          relation: "legacy_v1",
        },
        facet: "Structure",
        rows: null,
        missing: true,
      },
    ],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· 1 no longer exist")).toBeVisible()
    await expect(canvas.getByText(/no longer in the catalog/)).toBeVisible()
    // Absent, not disabled: a dead control invites a click that does nothing.
    await expect(
      canvas.queryByRole("button", { name: /Open shop\.public\.legacy_v1/ })
    ).toBeNull()
  },
}

/** Nothing was read: there is no empty « Sources » frame to load. */
export const Empty: Story = {
  args: { sources: [] },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

export const ThirtySourcesInANarrowPanel: Story = {
  args: {
    sources: Array.from({ length: 30 }, (_, index) => ({
      key: `many-${index}`,
      address: {
        catalog: "warehouse",
        namespace: "analytics",
        relation: `fact_sales_by_region_and_channel_${index}`,
      },
      facet: index % 2 === 0 ? "Structure" : "Indexes",
      rows: index % 3 === 0 ? index * 7 : null,
    })),
  },
  decorators: [
    (Story) => (
      <div className="w-[320px] border p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Oxyn read 30 objects")).toBeVisible()
    const list = canvasElement.querySelector("ul")
    await expect(list!.scrollWidth).toBeLessThanOrEqual(list!.clientWidth)
  },
}

/**
 * An object name is server input: it is shown, never interpreted, and a
 * right-to-left name does not reorder the address around it.
 */
export const HostileAndRightToLeftNames: Story = {
  args: {
    sources: [
      {
        key: "h1",
        address: {
          catalog: null,
          namespace: "public",
          relation: 'users"; DROP TABLE audit; --',
        },
        facet: "DDL",
        rows: null,
      },
      {
        key: "h2",
        address: { catalog: null, namespace: "عام", relation: "الفواتير" },
        facet: "Structure",
        rows: 12,
      },
      {
        key: "h3",
        address: {
          catalog: null,
          namespace: "public",
          relation: `a_very_long_${"segment_".repeat(12)}name`,
        },
        facet: "Data",
        rows: 1,
      },
    ],
  },
  decorators: [
    (Story) => (
      <div className="w-[320px] border p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText('public.users"; DROP TABLE audit; --')
    ).toBeVisible()
    // Singular when there is one row: « 1 rows » reads as a bug.
    await expect(
      canvas.getByText("1 row read · their values stayed on this machine")
    ).toBeVisible()
    const list = canvasElement.querySelector("ul")
    await expect(list!.scrollWidth).toBeLessThanOrEqual(list!.clientWidth)
  },
}

export const Folded: Story = {
  args: { defaultOpen: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.queryByRole("list")).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: /Oxyn read 3 objects/ })
    )
    await expect(canvas.getByRole("list")).toBeVisible()
  },
}

/**
 * Both kinds of row line, and a vanished object, in the light theme.
 *
 * Storybook renders dark by default: without this story axe would never check
 * this component's text contrast in light.
 */
export const UnderMetadataInLight: Story = {
  globals: { theme: "light" },
  args: {
    sources: [
      ...SOURCES,
      {
        key: "gone",
        address: { catalog: "shop", namespace: "public", relation: "legacy" },
        facet: "Structure",
        rows: null,
        missing: true,
      },
    ],
  },
}
