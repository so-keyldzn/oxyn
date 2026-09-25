import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test"

import { QuickOpen } from "./quick-open"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"

const hit = (
  relation: string,
  kind = "table",
  namespace = "public"
): CatalogSearchHit => ({
  address: { catalog: null, namespace, relation },
  name: relation,
  kind,
  holdsRecords: true,
  matched: "relationName",
  matchedFields: [],
})

const meta = {
  title: "Oxyn/QuickOpen",
  component: QuickOpen,
  args: {
    open: true,
    onOpenChange: fn(),
    query: "",
    onQueryChange: fn(),
    results: { status: "idle" },
    onOpenObject: fn(),
    onRetry: fn(),
  },
} satisfies Meta<typeof QuickOpen>

export default meta
type Story = StoryObj<typeof meta>

/** Before a name is typed: the scope is said, nothing is read. */
export const Initial: Story = {
  play: async () => {
    const dialog = await screen.findByRole("dialog", { name: "Open object" })
    await expect(
      within(dialog).getAllByText(
        "Searches the objects already loaded in the catalog."
      ).length
    ).toBeGreaterThan(0)
    await expect(within(dialog).getByRole("status")).toHaveTextContent(
      "Type part of a name."
    )
  },
}

export const Searching: Story = {
  args: { query: "ord", results: { status: "searching" } },
}

/** Enter opens the selected object, as a click in the catalog does. */
export const Populated: Story = {
  args: {
    query: "ord",
    results: {
      status: "done",
      hits: [hit("orders"), hit("order_lines"), hit("orders_by_day", "view")],
    },
  },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("dialog", { name: "Open object" })
    await expect(
      await within(dialog).findByRole("option", { name: /orders_by_day/ })
    ).toHaveTextContent("view · public")
    await userEvent.click(
      within(dialog).getByPlaceholderText("Open a table, a view…")
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onOpenObject).toHaveBeenCalledWith(
      expect.objectContaining({ name: "orders" })
    )
  },
}

export const NothingLoadedMatches: Story = {
  args: { query: "invoices", results: { status: "done", hits: [] } },
  play: async () => {
    const status = await screen.findByText("No loaded object matches.")
    // After the dialog's opening fade.
    await waitFor(() => expect(status).toBeVisible())
  },
}

export const Failed: Story = {
  args: {
    query: "ord",
    results: {
      status: "error",
      error: {
        message: "The catalog of this connection is closed",
        retryable: false,
      },
    },
  },
  play: async () => {
    const title = await screen.findByText("Cannot search the catalog")
    await waitFor(() => expect(title).toBeVisible())
  },
}
