import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { MentionChip } from "./assistant-mention-chip"
import type { MentionKind } from "./assistant-mention-chip"

const meta = {
  title: "Oxyn/Assistant/Mention chip",
  component: MentionChip,
  args: { kind: "table", label: "orders" },
  decorators: [
    (Story) => (
      <p className="max-w-xl p-4 text-sm">
        Count the rows of <Story /> per month.
      </p>
    ),
  ],
} satisfies Meta<typeof MentionChip>

export default meta
type Story = StoryObj<typeof meta>

/** As the composer shows it: inline, not a control. */
export const InTheField: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.queryByRole("button")).toBeNull()
    await expect(canvas.getByText("@orders")).toBeVisible()
  },
}

/** Every kind has its icon; the word is in the title, not in the icon. */
export const EveryKind: Story = {
  render: () => (
    <>
      {(
        [
          ["table", "orders"],
          ["view", "open_orders"],
          ["collection", "events"],
          ["column", "orders.status"],
          ["savedQuery", "Monthly revenue"],
          ["other", "orders_idx"],
        ] satisfies Array<[MentionKind, string]>
      ).map(([kind, label]) => (
        <MentionChip key={kind} kind={kind} label={label} />
      ))}
    </>
  ),
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelectorAll("[data-slot=assistant-mention] svg")
    ).toHaveLength(6)
  },
}

/** In the thread: a button that opens the object. */
export const InTheThread: Story = {
  args: { kind: "column", label: "orders.status", onOpen: fn() },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Open column orders.status" })
    )
    await expect(args.onOpen).toHaveBeenCalledOnce()
  },
}

/** Gone since: dimmed, said in words, and not a control. */
export const NotFound: Story = {
  args: { missing: true, onOpen: fn() },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· not found")).toBeVisible()
    await expect(canvas.queryByRole("button")).toBeNull()
  },
}

/** A long name is cut by the chip, never by the line. */
export const LongName: Story = {
  args: {
    label:
      "customer_lifetime_value_by_acquisition_channel_and_region_monthly_snapshot",
  },
  decorators: [
    (Story) => (
      <div className="w-[240px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    await expect(canvasElement.scrollWidth).toBeLessThanOrEqual(
      canvasElement.clientWidth
    )
  },
}

/** Its own context menu: `Open object`, the button's action, and nothing else. */
export const ContextMenuInTheThread: Story = {
  args: { kind: "table", label: "orders", onOpen: fn() },
  play: async ({ canvasElement, args }) => {
    const page = within(document.body)
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: within(canvasElement).getByRole("button", {
        name: "Open table orders",
      }),
    })
    await page.findByRole("menu")
    await expect(
      page.getAllByRole("menuitem").map((item) => item.textContent.trim())
    ).toEqual(["Open object"])
    await userEvent.click(page.getByRole("menuitem", { name: "Open object" }))
    await expect(args.onOpen).toHaveBeenCalledOnce()
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}
