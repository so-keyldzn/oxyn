import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within, waitFor } from "storybook/test"
import { ExplorerConnectionList } from "./explorer-connection-list"
import { ExplorerConnectionRow } from "./explorer-connection-row"
import { CatalogSourceTools } from "./catalog-source-tools"
import { CatalogPanel } from "./catalog-panel"
import {
  SidebarProvider,
  useSidebar,
  SidebarMenuItem,
  SidebarMenuButton,
} from "@/components/ui/sidebar"
import { HugeiconsIcon } from "@hugeicons/react"
import { Settings01Icon } from "@hugeicons/core-free-icons"
import { catalog } from "./fixtures"
import type {
  ExplorerConnection,
  MultiBaseExplorerProps,
} from "./multi-base-explorer"
import { MultiBaseExplorer } from "./multi-base-explorer"

const group = {
  id: "example-server",
  name: "Development server",
  expanded: true,
  seed: "Orders",
  members: [
    {
      id: "example-orders",
      name: "Orders",
      location: "localhost / orders",
      driver: "postgres",
      environment: "development" as const,
      readOnly: true,
      state: "active" as const,
      transaction: "Transaction open in a console",
    },
  ],
  discovery: {
    state: "ready" as const,
    databases: [
      { name: "orders", can_connect: true },
      { name: "audit", can_connect: false },
    ],
  },
}
const meta = {
  title: "Oxyn/MultiBaseExplorer",
  component: MultiBaseExplorer,
  decorators: [
    (Story) => (
      <div className="w-80">
        <Story />
      </div>
    ),
  ],
  args: {
    groups: [group],
    ungrouped: [],
    selected: null,
    loading: false,
    error: null,
    canGroup: true,
    onSelect: fn(),
    onToggle: fn(),
    onActivate: fn(),
    onCancel: fn(),
    onDiscover: fn(),
    onDatabase: fn(),
    onCreateGroup: fn(),
    onRemoveGroup: fn(),
    onRenameGroup: fn(),
    onDisconnectGroup: fn(),
    onDisconnect: fn(),
    onUnlink: fn(),
    onAttach: fn(),
    onRefresh: fn(),
  },
} satisfies Meta<typeof MultiBaseExplorer>
export default meta
type Story = StoryObj<typeof meta>
export const Loaded: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(
      await canvas.findByRole("button", { name: /Orders — active/ })
    )
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onSelect).toHaveBeenCalledWith(
      "connection:example-orders"
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onActivate).toHaveBeenCalledWith("example-orders")
  },
}
export const Initial: Story = {
  args: {
    groups: [
      {
        ...group,
        expanded: false,
        discovery: { state: "initial", databases: [] },
      },
    ],
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Expand Development server" })
    )
    await expect(args.onToggle).toHaveBeenCalledWith(group.id)
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onDiscover).not.toHaveBeenCalled()
  },
}
export const Empty: Story = {
  args: { groups: [], ungrouped: [], canGroup: false },
}
export const Loading: Story = {
  args: { loading: true, groups: [], canGroup: false },
}
export const Discovering: Story = {
  args: {
    groups: [{ ...group, discovery: { state: "loading", databases: [] } }],
  },
}
export const EmptyServer: Story = {
  args: {
    groups: [{ ...group, discovery: { state: "ready", databases: [] } }],
  },
}
export const DiscoveryError: Story = {
  args: {
    groups: [
      {
        ...group,
        discovery: {
          state: "error",
          databases: [],
          error: {
            message: "Example server refused catalog access.",
            retryable: false,
          },
        },
      },
    ],
  },
}
export const ReadError: Story = {
  args: {
    groups: [],
    error: {
      message: "Example workspace could not be read.",
      retryable: false,
    },
    canGroup: false,
  },
}
export const Opening: Story = {
  args: {
    groups: [
      { ...group, members: [{ ...group.members[0]!, state: "opening" }] },
    ],
  },
}
export const Cancelling: Story = {
  args: {
    groups: [
      { ...group, members: [{ ...group.members[0]!, state: "cancelling" }] },
    ],
  },
}

export const GroupActionsDoNotActivate: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.queryByRole("button", { name: "Rename group" })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Manage Development server" })
    )
    const menu = within(document.body)
    await userEvent.click(
      await menu.findByRole("menuitem", { name: "Rename group" })
    )
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "Manage Development server" })
      ).toHaveAttribute("aria-expanded", "false")
    )
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "Manage Development server" })
      ).toHaveFocus()
    )
    await waitFor(() =>
      expect(menu.queryByRole("menu", { hidden: true })).toBeNull()
    )
    await expect(args.onRenameGroup).toHaveBeenCalledWith(group.id)
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onDiscover).not.toHaveBeenCalled()
    await expect(args.onToggle).not.toHaveBeenCalled()
  },
}

const manyConnections = (count: number): Array<ExplorerConnection> =>
  Array.from({ length: count }, (_, index) => ({
    id: `dense-${index}`,
    name:
      index === 1 || index === 2
        ? "Orders"
        : index === 3
          ? "Very long analytics destination with a distinct original identity"
          : `Connection ${index + 1}`,
    location: `loopback / database_${index}`,
    driver: "sqlite",
    environment: "local",
    readOnly: false,
    state: index === 0 ? "active" : index < 4 ? "open" : "saved",
  }))
const largeCatalog = Array.from({ length: 500 }, (_, index) => ({
  ...catalog[0]!.children[0]!,
  name: `table_${index + 1}`,
  address: {
    catalog: "synthetic",
    namespace: "main",
    relation: `table_${index + 1}`,
  },
}))
function VolumeSidebar(args: MultiBaseExplorerProps) {
  const { state } = useSidebar()
  const catalogRegion = React.useRef<HTMLDivElement>(null)
  return (
    <div
      className="group flex h-[850px] min-w-0 flex-col border"
      style={{ width: state === "collapsed" ? 82 : 320 }}
      data-collapsible={state === "collapsed" ? "icon" : ""}
      data-testid="sidebar-volume"
    >
      <MultiBaseExplorer
        {...args}
        onSkipToCatalog={() =>
          catalogRegion.current
            ?.querySelector<HTMLInputElement>("input")
            ?.focus()
        }
      />
      <div ref={catalogRegion} className="flex min-h-0 flex-1 flex-col">
        <CatalogPanel
          embeddedNavigation
          connectionName="Connection 1"
          driver="sqlite"
          supported
          nodes={largeCatalog}
          loading={new Set()}
          selected={null}
          expanded={new Set()}
          onExpandedChange={fn()}
          onExpand={fn()}
          onSelect={fn()}
          search={{ hits: null, searching: false, onQuery: fn() }}
          refreshing={false}
          onRefresh={fn()}
          onCancelRefresh={fn()}
          problem={null}
          onLeave={fn()}
        />
      </div>
      <CatalogSourceTools
        onSchemaMap={fn()}
        onNativeTransfers={fn()}
        preferences={
          <SidebarMenuItem className="min-w-0 flex-1">
            <SidebarMenuButton aria-label="Metadata preferences">
              <HugeiconsIcon icon={Settings01Icon} strokeWidth={2} />
              <span className="sr-only">Metadata preferences</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        }
      />
    </div>
  )
}
const volumeRender: Story["render"] = (args) => (
  <SidebarProvider defaultOpen className="min-h-0">
    <VolumeSidebar {...args} />
  </SidebarProvider>
)
const volumePlay: Story["play"] = async ({ canvas, canvasElement, args }) => {
  const root = canvas.getByTestId("sidebar-volume")
  const connections = canvas.getByRole("region", {
    name: "Multi-base explorer",
  })
  const bounds = root.getBoundingClientRect()
  expect(connections.getBoundingClientRect().height).toBeLessThan(
    bounds.height * 0.34
  )
  const content = canvasElement.querySelector('[data-slot="sidebar-content"]')!
  expect(content.getBoundingClientRect().height).toBeGreaterThan(
    bounds.height * 0.5
  )
  await waitFor(() =>
    expect(
      canvasElement.querySelectorAll("[data-connection-row]").length
    ).toBeGreaterThan(0)
  )
  expect(
    canvasElement.querySelectorAll("[data-connection-row]").length
  ).toBeLessThan(40)
  const row = canvas.getByRole("button", { name: "Connection 1 — active" })
  expect(row.getBoundingClientRect().height).toBeLessThanOrEqual(36)
  await expect(row).toHaveAttribute("aria-current", "page")
  await expect(
    canvas.getByRole("button", { name: "Orders — open", pressed: true })
  ).toHaveAttribute("aria-pressed", "true")
  await expect(
    canvas.queryByRole("button", { name: "Switch connection" })
  ).toBeNull()
  await userEvent.click(
    canvas.getByRole("button", { name: "Find a saved connection" })
  )
  await userEvent.type(
    canvas.getByRole("textbox", { name: "Filter saved connections" }),
    "database_99"
  )
  if (args.ungrouped.length >= 100)
    await expect(
      canvas.getByRole("button", { name: "Connection 100 — saved" })
    ).toBeVisible()
  await expect(args.onRefresh).not.toHaveBeenCalled()
  await expect(args.onDiscover).not.toHaveBeenCalled()
  await userEvent.keyboard("{Escape}")
  await expect(
    canvas.getByRole("button", { name: "Find a saved connection" })
  ).toHaveFocus()
  expect(root.scrollWidth).toBeLessThanOrEqual(root.clientWidth)
}
export const ThirtyConnections: Story = {
  args: {
    groups: [],
    ungrouped: manyConnections(30),
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
  play: volumePlay,
}
export const HundredConnections: Story = {
  args: {
    groups: [],
    ungrouped: manyConnections(100),
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
  play: volumePlay,
}
export const CompactConnectionRow: Story = {
  render: (args) => (
    <ul className="w-64">
      <ExplorerConnectionRow
        connection={manyConnections(4)[3]!}
        selected
        grouped
        actions={args}
      />
    </ul>
  ),
  play: async ({ canvas, args }) => {
    const row = canvas.getByRole("button", {
      name: /Very long analytics.*— open/,
    })
    await userEvent.click(row)
    await expect(args.onActivate).not.toHaveBeenCalled()
    await userEvent.keyboard("{Enter}")
    await expect(args.onActivate).toHaveBeenCalledWith("dense-3")
    await userEvent.click(
      canvas.getByRole("button", { name: /Actions for Very long analytics/ })
    )
    const page = within(document.body)
    const action = await page.findByRole("menuitem", {
      name: "Switch workspace",
    })
    await waitFor(() => expect(action).toBeVisible())
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: /Actions for Very long analytics/ })
      ).toHaveAttribute("aria-expanded", "false")
    )
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: /Actions for Very long analytics/ })
      ).toHaveFocus()
    )
    await waitFor(() =>
      expect(page.queryByRole("menu", { hidden: true })).toBeNull()
    )
  },
}

export const ThousandConnections: Story = {
  args: {
    groups: [],
    ungrouped: manyConnections(1000),
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
  play: volumePlay,
}
export const CollapsedSidebarTools: Story = {
  args: { groups: [], ungrouped: manyConnections(30), canGroup: false },
  render: (args) => (
    <SidebarProvider defaultOpen={false} className="min-h-0">
      <VolumeSidebar {...args} />
    </SidebarProvider>
  ),
  play: async ({ canvas }) => {
    const frame = canvas.getByTestId("sidebar-volume")
    const bounds = frame.getBoundingClientRect()
    await expect(
      canvas.queryByRole("region", { name: "Multi-base explorer" })
    ).toBeNull()
    for (const name of [
      "View schema map",
      "Database transfers",
      "Metadata preferences",
    ]) {
      const button = canvas.getByRole("button", { name })
      const rect = button.getBoundingClientRect()
      expect(rect.left).toBeGreaterThanOrEqual(bounds.left)
      expect(rect.right).toBeLessThanOrEqual(bounds.right)
      expect(rect.bottom).toBeLessThanOrEqual(bounds.bottom)
    }
    await userEvent.click(canvas.getByRole("button", { name: "Catalog" }))
    await expect(
      canvas.getByRole("region", { name: "Multi-base explorer" })
    ).toBeVisible()
  },
}

export const ThousandConnectionsSnapshot: Story = {
  args: {
    groups: [],
    ungrouped: manyConnections(1000),
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
}
export const ThousandConnectionsKeyboardRoute: Story = {
  args: {
    groups: [],
    ungrouped: manyConnections(1000),
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
  play: async ({ canvas, args }) => {
    const find = canvas.getByRole("button", { name: "Find a saved connection" })
    find.focus()
    await userEvent.tab()
    await expect(
      canvas.getByRole("button", { name: "Refresh list" })
    ).toHaveFocus()
    await userEvent.tab()
    await expect(
      canvas.getByRole("button", { name: "Skip to database objects" })
    ).toHaveFocus()
    await userEvent.keyboard("{Enter}")
    await expect(
      canvas.getByRole("textbox", { name: "Filter loaded objects" })
    ).toHaveFocus()
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onRefresh).not.toHaveBeenCalled()
    await expect(args.onDiscover).not.toHaveBeenCalled()
  },
}

export const GroupedThousandConnections: Story = {
  args: {
    groups: [
      {
        ...group,
        members: manyConnections(1000),
        discovery: { state: "initial", databases: [] },
      },
    ],
    ungrouped: [],
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
  play: async ({ canvas, canvasElement, args }) => {
    await waitFor(() =>
      expect(
        canvasElement.querySelectorAll("[data-connection-row]").length
      ).toBeGreaterThan(0)
    )
    expect(
      canvasElement.querySelectorAll("[data-connection-row]").length
    ).toBeLessThan(40)
    await userEvent.click(
      canvas.getByRole("button", { name: "Find a saved connection" })
    )
    await userEvent.type(
      canvas.getByRole("textbox", { name: "Filter saved connections" }),
      "database_999"
    )
    await expect(
      await canvas.findByRole("button", { name: "Connection 1000 — saved" })
    ).toBeVisible()
    await expect(args.onDiscover).not.toHaveBeenCalled()
    await expect(args.onActivate).not.toHaveBeenCalled()
  },
}
export const VirtualConnectionKeyboardAndMenu: Story = {
  render: (args) => (
    <ExplorerConnectionList
      connections={manyConnections(1000)}
      selected="connection:dense-1"
      actions={args}
      nameCounts={new Map([["Orders", 2]])}
      label="Virtual saved destinations"
      filterKey=""
    />
  ),
  play: async ({ canvas, canvasElement, args }) => {
    const first = await canvas.findByRole("button", {
      name: "Connection 1 — active",
    })
    first.focus()
    await userEvent.keyboard("{End}")
    const last = await canvas.findByRole("button", {
      name: "Connection 1000 — saved",
    })
    await expect(last).toHaveFocus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onActivate).toHaveBeenCalledWith("dense-999")
    await userEvent.keyboard("{Home}")
    await expect(
      await canvas.findByRole("button", { name: "Connection 1 — active" })
    ).toHaveFocus()
    const trigger = canvas.getByRole("button", {
      name: "Actions for Connection 1",
    })
    await userEvent.click(trigger)
    await within(document.body).findByRole("menuitem", {
      name: "Return to workspace",
    })
    const viewport = canvas.getByRole("region", {
      name: "Virtual saved destinations",
    })
    viewport.scrollTop = viewport.scrollHeight
    viewport.dispatchEvent(new Event("scroll"))
    await waitFor(() =>
      expect(
        canvasElement.querySelector('[data-connection-row="dense-999"]')
      ).not.toBeNull()
    )
    await expect(trigger).toBeInTheDocument()
    expect(
      canvasElement.querySelectorAll("[data-connection-row]").length
    ).toBeLessThan(40)
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(trigger).toHaveFocus())
    await waitFor(() =>
      expect(
        within(document.body).queryByRole("menu", { hidden: true })
      ).toBeNull()
    )
  },
}

export const OffscreenActiveAndSelectedConnection: Story = {
  render: (args) => {
    const connections = manyConnections(1000).map((connection, index) => ({
      ...connection,
      state:
        index === 999
          ? ("active" as const)
          : index === 0
            ? ("open" as const)
            : connection.state,
    }))
    return (
      <ExplorerConnectionList
        connections={connections}
        selected="connection:dense-999"
        actions={args}
        nameCounts={new Map([["Orders", 2]])}
        label="Saved destination identities"
        filterKey=""
      />
    )
  },
  play: async ({ canvas, args }) => {
    const first = await canvas.findByRole("button", {
      name: "Connection 1 — open",
    })
    first.focus()
    await userEvent.keyboard("{End}")
    const target = await canvas.findByRole("button", {
      name: "Connection 1000 — active",
    })
    await expect(target).toHaveFocus()
    await expect(target).toHaveAttribute("aria-current", "page")
    await expect(target).toHaveAttribute("aria-pressed", "true")
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onSelect).not.toHaveBeenCalled()
    await userEvent.keyboard("{Enter}")
    await expect(args.onActivate).toHaveBeenCalledWith("dense-999")
  },
}

const distributedConnections = manyConnections(1000)
const distributedGroups = Array.from({ length: 100 }, (_, index) => ({
  ...group,
  id: `distributed-${index}`,
  name: `Server ${index + 1}`,
  members: distributedConnections.slice(index * 10, index * 10 + 10),
  discovery: { state: "initial" as const, databases: [] },
}))
export const DistributedThousandConnectionsSnapshot: Story = {
  args: {
    groups: distributedGroups,
    ungrouped: [],
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
}

export const DistributedThousandConnections: Story = {
  args: {
    groups: distributedGroups,
    ungrouped: [],
    selected: "connection:dense-1",
    canGroup: false,
  },
  render: volumeRender,
  play: async ({ canvas, canvasElement, args }) => {
    await waitFor(() =>
      expect(
        canvasElement.querySelectorAll("[data-connection-row]").length
      ).toBeGreaterThan(0)
    )
    expect(
      canvasElement.querySelectorAll("[data-connection-row]").length
    ).toBeLessThan(50)
    const first = await canvas.findByRole("button", {
      name: "Connection 1 — active",
    })
    await expect(first).toHaveAttribute("aria-current", "page")
    const viewport = canvas.getByRole("region", { name: /^Saved connections$/ })
    viewport.scrollTop = viewport.scrollHeight
    viewport.dispatchEvent(new Event("scroll"))
    await waitFor(() =>
      expect(
        canvasElement.querySelector('[data-connection-row="dense-999"]')
      ).not.toBeNull()
    )
    expect(
      canvasElement.querySelectorAll("[data-connection-row]").length
    ).toBeLessThan(50)
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onDiscover).not.toHaveBeenCalled()
    await userEvent.click(
      canvas.getByRole("button", { name: "Find a saved connection" })
    )
    await userEvent.type(
      canvas.getByRole("textbox", { name: "Filter saved connections" }),
      "database_999"
    )
    await expect(
      await canvas.findByRole("button", { name: "Connection 1000 — saved" })
    ).toBeVisible()
  },
}
