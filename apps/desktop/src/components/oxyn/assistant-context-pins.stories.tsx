import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantContextPins } from "./assistant-context-pins"
import type { ContextPin } from "./assistant-context-pins"

const PINS: Array<ContextPin> = [
  { key: "p1", kind: "object", label: "public.invoices", shape: "12 columns" },
  { key: "p2", kind: "result", label: "Invoices by month", shape: "200 rows" },
  { key: "p3", kind: "selection", label: "3 selected rows", shape: "3 rows" },
]

const meta = {
  title: "Oxyn/Assistant/ContextPins",
  component: AssistantContextPins,
  args: { pins: PINS, tier: "metadata", onRemove: fn() },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantContextPins>

export default meta
type Story = StoryObj<typeof meta>

/**
 * The default tier is not « nothing leaves »: the structure goes, and the
 * strip says so before the question is sent (ADR-0006).
 */
export const UnderMetadata: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const consequences = canvas
      .getAllByRole("listitem")
      .map((item) => item.textContent)
    await expect(consequences).toEqual([
      "Objects — its name, columns, types and indexes",
      "Results — column names and the number of rows; the values stay",
      "Selections — column names and the number of rows; the values stay",
    ])
    // Declarative: removing is local, and nothing here sends anything.
    await userEvent.click(
      canvas.getByRole("button", {
        name: "Remove public.invoices from this question's context",
      })
    )
    await expect(args.onRemove).toHaveBeenCalledWith("p1")
  },
}

/** `Local` promises that nothing leaves, and it is said for every kind. */
export const UnderLocal: Story = {
  args: { tier: "local" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const consequences = canvas
      .getAllByRole("listitem")
      .map((item) => item.textContent)
    await expect(consequences).toEqual([
      "Objects — nothing — only a local model reads its structure",
      "Results — nothing — only a local model reads it",
      "Selections — nothing — only a local model reads it",
    ])
    await expect(canvas.getByText("Local")).toBeVisible()
  },
}

/** The only tier where a value may leave, and only one you approved. */
export const UnderSampled: Story = {
  args: { tier: "sampled" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const consequences = canvas
      .getAllByRole("listitem")
      .map((item) => item.textContent)
    await expect(
      consequences.filter((line) =>
        line.includes("the values you approve, column by column")
      )
    ).toHaveLength(2)
    await expect(consequences[0]).toBe(
      "Objects — its name, columns, types and indexes"
    )
  },
}

/** Nothing attached: no strip at all above the composer. */
export const Empty: Story = {
  args: { pins: [] },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

/** One kind pinned, one line of consequence: no irrelevant sentence. */
export const OneObjectOnly: Story = {
  args: { pins: [PINS[0]!] },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getAllByRole("listitem")).toHaveLength(1)
    await expect(canvas.queryByText(/the values stay/)).toBeNull()
  },
}

export const ManyPinsInANarrowPanel: Story = {
  args: {
    pins: [
      ...PINS,
      ...Array.from({ length: 7 }, (_, index) => ({
        key: `extra-${index}`,
        kind: "object" as const,
        label: `analytics.fact_sales_by_region_and_channel_${index}`,
        shape: `${index + 4} columns`,
      })),
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
    const section = canvasElement.querySelector(
      '[data-slot="assistant-context-pins"]'
    ) as HTMLElement
    // The pins scroll inside their own strip; the panel around them does not.
    await expect(section.scrollWidth).toBeLessThanOrEqual(section.clientWidth)
  },
}

/**
 * A pinned object name is server input. It is shown as text, kept to one
 * line, and its direction is isolated so an Arabic name does not reorder the
 * strip around it.
 */
export const HostileAndRightToLeftLabels: Story = {
  args: {
    pins: [
      {
        key: "h1",
        kind: "object",
        label: 'public.users"; DROP TABLE audit; --',
        shape: "4 columns",
      },
      { key: "h2", kind: "object", label: "عام.الفواتير", shape: "9 columns" },
      {
        key: "h3",
        kind: "result",
        label: "<script>alert(1)</script>",
        shape: "1 row",
      },
    ],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvasElement.querySelector("script")).toBeNull()
    await expect(canvas.getByText("<script>alert(1)</script>")).toBeVisible()
    // The full name stays reachable without widening the pin.
    await expect(
      canvas.getByTitle('public.users"; DROP TABLE audit; --')
    ).toBeVisible()
  },
}

/**
 * All three kinds of pin in the light theme.
 *
 * Storybook renders dark by default: without this story axe would never check
 * the consequence lines — the one text users must read before sending — in
 * light.
 */
export const UnderMetadataInLight: Story = {
  globals: { theme: "light" },
}
