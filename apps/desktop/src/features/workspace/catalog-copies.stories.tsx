import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { CatalogPanel } from "@/components/oxyn/catalog-panel"
import { addressKey } from "@/components/oxyn/catalog-tree"
import { catalog } from "@/components/oxyn/fixtures"
import { SidebarProvider } from "@/components/ui/sidebar"
import { copyToClipboard } from "@/features/metadata/clipboard"

const INSERT =
  'INSERT INTO "billing"."public"."invoices" ("id", "amount")\nVALUES (?, ?);'

/**
 * What a copy from the catalog says, where it says it: the toast of the
 * copy, never the catalog's own alert. The text arrives after the click, as
 * the backend's would (`writeClipboard`); the clipboard is a stand-in, the
 * test browser's own being refused to an unfocused page.
 */
const meta = {
  title: "Screens/CatalogCopies",
  component: CatalogPanel,
  decorators: [
    (Story) => (
      <SidebarProvider defaultOpen className="min-h-0">
        <div
          data-sidebar="sidebar"
          className="flex h-[520px] w-[280px] flex-col bg-sidebar text-sidebar-foreground"
        >
          <Story />
        </div>
      </SidebarProvider>
    ),
  ],
  beforeEach: () => {
    const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard")
    Object.defineProperty(navigator, "clipboard", {
      value: {
        write: async (items: Array<ClipboardItem>) => {
          for (const item of items) await item.getType("text/plain")
        },
        writeText: async () => undefined,
      },
      configurable: true,
    })
    return () => {
      if (clipboard) Object.defineProperty(navigator, "clipboard", clipboard)
      else Reflect.deleteProperty(navigator, "clipboard")
    }
  },
  args: {
    connectionName: "billing",
    driver: "PostgreSQL",
    supported: true,
    nodes: catalog,
    loading: new Set<string>(),
    selected: null,
    expanded: new Set(catalog[0] ? [addressKey(catalog[0].address)] : []),
    onExpandedChange: fn(),
    onExpand: fn(),
    onSelect: fn(),
    search: { hits: null, searching: false, onQuery: fn() },
    refreshing: false,
    onRefresh: fn(),
    onCancelRefresh: fn(),
    problem: null,
    onDismissProblem: fn(),
    onOpen: fn(),
    onCopyName: fn(),
    onRefreshLevel: fn(),
    onLeave: fn(),
  },
} satisfies Meta<typeof CatalogPanel>

export default meta
type Story = StoryObj<typeof meta>

async function copyInsertTemplate(canvas: ReturnType<typeof within>) {
  await userEvent.pointer({
    keys: "[MouseRight]",
    target: canvas.getByText("invoices"),
  })
  const body = within(document.body)
  await userEvent.click(await body.findByRole("menuitem", { name: /Copy as/ }))
  await userEvent.click(
    await body.findByRole("menuitem", { name: "INSERT template" })
  )
  return body
}

/** The template is composed after the click, and the copy says it worked. */
export const CopiedAsInsert: Story = {
  args: {
    copyAs: {
      definition: true,
      onCopy: (_node, _form) =>
        void copyToClipboard(
          new Promise((resolve) => window.setTimeout(resolve, 50, INSERT)),
          "INSERT template"
        ),
    },
  },
  play: async ({ canvas }) => {
    const body = await copyInsertTemplate(canvas)
    await expect(await body.findByText("INSERT template copied")).toBeVisible()
  },
}

/**
 * A copy that could not be composed says so as a copy: the catalog shows no
 * alert, and « Refresh » is not offered as the way out.
 */
export const CopyFailureStaysACopy: Story = {
  args: {
    copyAs: {
      definition: true,
      onCopy: (_node, _form) =>
        void copyToClipboard(
          Promise.reject(
            new Error("The connection policy forbids reading this schema.")
          ),
          "INSERT template"
        ),
    },
  },
  play: async ({ canvas }) => {
    const body = await copyInsertTemplate(canvas)
    await expect(
      await body.findByText("INSERT template not copied")
    ).toBeVisible()
    await expect(
      body.getByText("The connection policy forbids reading this schema.")
    ).toBeVisible()
    await waitFor(() => expect(canvas.queryByRole("alert")).toBeNull())
    await expect(canvas.queryByText(/The catalog could not be read/)).toBeNull()
  },
}
