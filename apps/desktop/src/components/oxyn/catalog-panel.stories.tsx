import type * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { CatalogPanel } from "./catalog-panel"
import { catalog } from "./fixtures"
import { HOSTILE, unusualCatalog } from "./metadata-fixtures"
import { SidebarProvider, useSidebar } from "@/components/ui/sidebar"

/**
 * The inside of `Sidebar`, framed the way the sidebar frames it — without its
 * mobile sheet, which the narrow test browser would otherwise pick. The
 * `group` carries `data-collapsible`, as the real sidebar does.
 */
function SidebarFrame({
  children,
  width = 280,
}: {
  children: React.ReactNode
  width?: number
}) {
  const { state } = useSidebar()
  return (
    <div
      className="group peer"
      data-state={state}
      data-collapsible={state === "collapsed" ? "icon" : ""}
    >
      <div
        data-sidebar="sidebar"
        className="flex h-[640px] flex-col bg-sidebar text-sidebar-foreground"
        style={{ width: state === "collapsed" ? 64 : width }}
      >
        {children}
      </div>
    </div>
  )
}

const meta = {
  title: "Oxyn/CatalogPanel",
  component: CatalogPanel,
  decorators: [
    (Story) => (
      <SidebarProvider defaultOpen className="min-h-0">
        <SidebarFrame>
          <Story />
        </SidebarFrame>
      </SidebarProvider>
    ),
  ],
  args: {
    connectionName: "billing",
    driver: "PostgreSQL",
    supported: true,
    nodes: catalog,
    loading: new Set<string>(),
    selected: null,
    expanded: new Set<string>(),
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

/** The header names the connection; it is not a button that does nothing. */
export const Browse: Story = {
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("billing")).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /billing/ })).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Refresh catalog" })
    )
    await expect(args.onRefresh).toHaveBeenCalled()
  },
}

/** The first read: skeleton rows, a status line and a way to cancel. */
export const FirstRead: Story = {
  args: { nodes: undefined, refreshing: true },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Reading the catalog…"
    )
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancelRefresh).toHaveBeenCalled()
  },
}

/**
 * A refresh over a loaded tree keeps the tree. Refresh and Cancel keep their
 * own place: the group action goes on meaning « refresh » and is inert, and
 * cancelling is the button in the status line — a button whose meaning flips
 * is a missed click away from restarting what was to be stopped.
 */
export const Refreshing: Story = {
  args: { refreshing: true },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByRole("tree", { name: "Catalog" })).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Refresh catalog" })
    ).toBeDisabled()
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancelRefresh).toHaveBeenCalled()
    await expect(args.onRefresh).not.toHaveBeenCalled()
  },
}

/** Cancel pressed, answer pending: said, and not pressable twice. */
export const Cancelling: Story = {
  args: { refreshing: true, cancelling: true },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Cancelling the refresh…"
    )
    await expect(canvas.getByRole("button", { name: "Cancel" })).toBeDisabled()
  },
}

export const Cancelled: Story = {
  args: { notice: "Refresh cancelled. The tree shows the last read." },
}

export const EmptyCatalog: Story = {
  args: { nodes: [] },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("No object loaded yet.")).toBeVisible()
  },
}

export const RetryableError: Story = {
  args: {
    nodes: undefined,
    problem: {
      kind: "error",
      message: "could not receive data from server: Connection reset by peer",
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("The catalog could not be read")
    ).toBeVisible()
    await expect(canvas.getByText(/refreshing may succeed/)).toBeVisible()
    const alert = canvas.getByRole("alert")
    await userEvent.click(
      within(alert).getByRole("button", { name: "Refresh catalog" })
    )
    await expect(args.onRefresh).toHaveBeenCalled()
  },
}

/** Not retryable: said so, and no button promises otherwise. */
export const Error: Story = {
  args: {
    problem: {
      kind: "error",
      message:
        "ERROR:  permission denied for schema reporting\nSQLSTATE: 42501",
      retryable: false,
    },
  },
  play: async ({ canvas, args }) => {
    const alert = canvas.getByRole("alert")
    await expect(alert).toHaveTextContent("will fail the same way")
    await expect(
      within(alert).queryByRole("button", { name: "Refresh catalog" })
    ).toBeNull()
    await userEvent.click(
      within(alert).getByRole("button", { name: "Dismiss" })
    )
    await expect(args.onDismissProblem).toHaveBeenCalled()
  },
}

export const Denied: Story = {
  args: {
    problem: {
      kind: "denied",
      reason: "Catalog introspection is disabled for billing (production).",
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Catalog read refused")).toBeVisible()
  },
}

export const NoCatalog: Story = {
  args: { supported: false, connectionName: "cache", driver: "Redis" },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("This database has no catalog to browse.")
    ).toBeVisible()
    await expect(canvas.queryByRole("tree")).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: "Refresh catalog" })
    ).toBeNull()
  },
}

/** Folded to the rail, the catalog is one icon, and that icon unfolds the sidebar. */
export const Rail: Story = {
  decorators: [
    (Story) => (
      <SidebarProvider defaultOpen={false} className="min-h-0">
        <SidebarFrame>
          <Story />
        </SidebarFrame>
      </SidebarProvider>
    ),
  ],
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("tree")).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Catalog" }))
    await waitFor(() =>
      expect(canvas.getByRole("tree", { name: "Catalog" })).toBeVisible()
    )
  },
}

export const UnusualNames: Story = {
  args: {
    nodes: unusualCatalog,
    connectionName: "إنتاج — قاعدة بيانات الفواتير الرئيسية للمنطقة",
    expanded: new Set([JSON.stringify(["billing", "public", null])]),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(HOSTILE)).toBeVisible()
    await expect(
      canvas.getByText("إنتاج — قاعدة بيانات الفواتير الرئيسية للمنطقة")
    ).toHaveAttribute("dir", "auto")
  },
}

export const Narrow: Story = {
  args: { nodes: unusualCatalog },
  decorators: [
    (Story) => (
      <SidebarProvider defaultOpen className="min-h-0">
        <SidebarFrame width={200}>
          <Story />
        </SidebarFrame>
      </SidebarProvider>
    ),
  ],
}
