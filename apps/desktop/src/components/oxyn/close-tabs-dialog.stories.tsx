import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test"

import { CloseTabsDialog } from "./close-tabs-dialog"

const reviewTab = (title: string, index: number) => ({
  key: String(index),
  title,
  blocked: null,
  reasons: {
    title,
    connection: "billing",
    unsaved: true,
    conflict: false,
    running: index === 0,
    transaction: index === 0 ? ("open" as const) : null,
    hasSavedCopy: true,
  },
})
const tabs = ["orders.sql", "invoices.sql"].map(reviewTab)
const meta = {
  title: "Oxyn/CloseTabsDialog",
  component: CloseTabsDialog,
  args: {
    review: { tabs, busy: false, notice: null },
    onCancel: fn(),
    onSave: fn(),
    onDiscard: fn(),
  },
} satisfies Meta<typeof CloseTabsDialog>
export default meta
type Story = StoryObj<typeof meta>

/** A large collective review keeps the decision buttons outside its scroller. */
export const LongReviewStaysBounded: Story = {
  args: {
    review: {
      tabs: Array.from({ length: 32 }, (_, index) => ({
        ...reviewTab(`query-${index + 1}.sql`, index),
        key: `review-${index}`,
        title: `query-${index + 1}.sql`,
      })),
      busy: false,
      notice: null,
    },
  },
  play: async () => {
    const dialog = await screen.findByRole("alertdialog")
    const list = within(dialog).getByRole("list", { name: "Tabs to close" })
    const viewport = list.closest<HTMLElement>(
      '[data-slot="scroll-area-viewport"]'
    )
    if (!viewport) throw new Error("Missing collective review viewport")
    await waitFor(() => {
      expect(viewport.clientHeight).toBeLessThanOrEqual(256)
      expect(viewport.scrollHeight).toBeGreaterThan(viewport.clientHeight)
    })
    const cancel = within(dialog).getByRole("button", { name: "Cancel" })
    const top = cancel.getBoundingClientRect().top
    viewport.scrollTop = viewport.scrollHeight
    await waitFor(() => {
      const last = within(list)
        .getByText("query-32.sql")
        .getBoundingClientRect()
      expect(last.bottom).toBeLessThanOrEqual(
        viewport.getBoundingClientRect().bottom
      )
      expect(cancel.getBoundingClientRect().top).toBe(top)
    })
  },
}

export const CollectiveReview: Story = {
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    const controls = within(dialog)
    await waitFor(() =>
      expect(controls.getByRole("button", { name: "Cancel" })).toHaveFocus()
    )
    await expect(dialog).toHaveTextContent("orders.sql")
    await expect(dialog).toHaveTextContent("invoices.sql")
    await expect(dialog).toHaveTextContent("A transaction is open on billing")
    controls.getByRole("list", { name: "Tabs to close" }).focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onDiscard).not.toHaveBeenCalled()
    await expect(args.onSave).not.toHaveBeenCalled()
    await userEvent.click(
      controls.getByRole("button", { name: "Discard all and close" })
    )
    await expect(args.onDiscard).toHaveBeenCalledOnce()
  },
}
export const Busy: Story = {
  args: { review: { tabs, busy: true, notice: null } },
  play: async ({ args }) => {
    const dialog = within(await screen.findByRole("alertdialog"))
    await expect(
      dialog.getByRole("button", { name: "Save all and close" })
    ).toBeDisabled()
    await expect(
      dialog.getByRole("button", { name: "Discard all and close" })
    ).toBeDisabled()
    await userEvent.click(dialog.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}
export const FailedSave: Story = {
  args: {
    review: {
      tabs,
      busy: false,
      notice: "Not saved: invoices.sql. No tabs were closed.",
    },
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "No tabs were closed"
    )
  },
}
export const BlockedExport: Story = {
  args: {
    review: {
      tabs: [
        {
          key: "result",
          title: "Query result",
          reasons: null,
          blocked: "Finish or cancel the export before closing this tab.",
        },
      ],
      busy: false,
      notice: null,
    },
  },
  play: async () => {
    const dialog = within(await screen.findByRole("alertdialog"))
    await expect(
      dialog.getByRole("button", { name: "Discard all and close" })
    ).toBeDisabled()
  },
}
