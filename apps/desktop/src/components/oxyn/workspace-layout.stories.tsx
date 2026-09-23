import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"
import { HugeiconsIcon } from "@hugeicons/react"
import { DatabaseIcon, InformationCircleIcon } from "@hugeicons/core-free-icons"

import { AssistantEntryButton } from "./assistant-entry-button"
import { enabledEntry } from "./assistant-fixtures"
import { ConsoleView } from "./console-view"
import { invoiceColumns, syntheticPages } from "./fixtures"
import { ObjectViewFrame } from "./object-view-frame"
import type { ObjectTab } from "./object-view-frame"
import { PreviewControls } from "./preview-controls"
import { ResultPanel } from "./result-panel"
import { StatusBar } from "./status-bar"
import { WorkspaceAside } from "./workspace-aside"
import { WorkspaceLayout } from "./workspace-layout"
import type { LeftView } from "./workspace-layout"
import { WorkspaceTabs, tabPanelValue } from "./workspace-tabs"
import type { WorkspaceTabItem } from "./workspace-tabs"
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { TabsContent } from "@/components/ui/tabs"

const tabs: Array<WorkspaceTabItem> = [
  {
    kind: "object",
    key: "object:invoices",
    title: "invoices",
    objectKind: "table",
  },
  {
    kind: "console",
    key: "console:1",
    title: "unpaid invoices.sql",
    fromAgent: false,
    activity: "idle",
    unsaved: true,
  },
  { kind: "result", key: "result:r1", title: "Result #12" },
]

function DemoSidebar() {
  return (
    <Sidebar variant="inset" collapsible="icon">
      <SidebarContent>
        <SidebarGroup>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton tooltip="invoices">
                <HugeiconsIcon icon={DatabaseIcon} strokeWidth={2} />
                <span>invoices</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>
    </Sidebar>
  )
}

const meta = {
  title: "Oxyn/WorkspaceLayout",
  component: WorkspaceLayout,
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <div className="h-[640px] w-[1280px] overflow-hidden border">
        <Story />
      </div>
    ),
  ],
  args: {
    sidebar: <DemoSidebar />,
    sidebarOpen: true,
    onSidebarOpenChange: fn(),
    leftView: "catalog",
    onLeftViewChange: fn(),
    connectionName: "billing primary",
    environment: "production",
    readOnly: false,
    tabs: null,
    activeTab: "console:1",
    onActiveTabChange: fn(),
    children: null,
    compact: false,
    onOpenSettings: fn(),
    onDisconnect: fn(),
    statusBar: null,
  },
  render: function Render(args, { parameters }) {
    const [active, setActive] = React.useState(args.activeTab)
    const [view, setView] = React.useState<LeftView>(args.leftView)
    const [asideOpen, setAsideOpen] = React.useState(args.asideOpen ?? false)
    const shown: Array<WorkspaceTabItem> = parameters.noTab === true ? [] : tabs
    return (
      <WorkspaceLayout
        {...args}
        leftView={view}
        onLeftViewChange={(next) => {
          setView(next)
          args.onLeftViewChange(next)
        }}
        activeTab={active}
        onActiveTabChange={(key) => {
          setActive(key)
          args.onActiveTabChange(key)
        }}
        asideOpen={asideOpen}
        onAsideOpenChange={args.aside ? setAsideOpen : undefined}
        tabs={
          <WorkspaceTabs
            tabs={shown}
            onClose={fn()}
            onNewConsole={fn()}
            onCancelOpening={fn()}
          />
        }
        statusBar={
          <StatusBar
            connectionName={args.connectionName}
            driver="postgres"
            environment={args.environment}
            readOnly={args.readOnly}
            execution={{ status: "idle" }}
          />
        }
      >
        {shown.length === 0
          ? (args.children ?? null)
          : shown.map((tab) => (
              <TabsContent
                key={tab.key}
                value={tab.key}
                keepMounted
                className="h-full p-3"
              >
                <label className="flex flex-col gap-1 text-sm">
                  Scratch for {tab.title}
                  <input
                    className="border px-2"
                    aria-label={`${tab.title} scratch`}
                  />
                </label>
              </TabsContent>
            ))}
      </WorkspaceLayout>
    )
  },
} satisfies Meta<typeof WorkspaceLayout>

export default meta
type Story = StoryObj<typeof meta>

/** Switching tabs keeps what a hidden tab holds: nothing is remounted. */
export const Wide: Story = {
  play: async ({ canvas, canvasElement }) => {
    await expect(
      within(barOf(canvasElement)).getByText("billing primary")
    ).toBeVisible()
    const scratch = canvas.getByLabelText("unpaid invoices.sql scratch")
    await userEvent.type(scratch, "kept")
    await userEvent.click(canvas.getByRole("tab", { name: /invoices$/ }))
    await userEvent.click(
      canvas.getByRole("tab", { name: /unpaid invoices\.sql/ })
    )
    await expect(
      canvas.getByLabelText("unpaid invoices.sql scratch")
    ).toHaveValue("kept")
  },
}

/** READ ONLY sits next to the connection name, in the bar itself. */
export const ReadOnly: Story = {
  args: { readOnly: true, environment: "staging" },
  play: async ({ canvasElement }) => {
    await expect(barOf(canvasElement)).toHaveTextContent("READ ONLY")
  },
}

/**
 * Below 1200 px the sidebar is a rail whatever the wide-screen preference;
 * toggling it there does not change that preference.
 */
export const Compact: Story = {
  args: { compact: true },
  decorators: [
    (Story) => (
      <div className="h-[640px] w-[980px] overflow-hidden border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, args, canvasElement }) => {
    const wrapper = canvasElement.querySelector('[data-slot="sidebar"]')
    await expect(wrapper).toHaveAttribute("data-state", "collapsed")
    await userEvent.click(
      canvas.getByRole("button", { name: "Toggle sidebar" })
    )
    await waitFor(() =>
      expect(wrapper).toHaveAttribute("data-state", "expanded")
    )
    await expect(args.onSidebarOpenChange).not.toHaveBeenCalled()
  },
}

/**
 * The tier is read next to `Ask AI`, in the connection bar, and a narrow
 * window keeps both: folding the badge away would let a question leave
 * before its tier was seen.
 */
export const CompactWithTheAssistantEntry: Story = {
  args: {
    compact: true,
    aiEntry: (
      <AssistantEntryButton
        entry={enabledEntry}
        tier="metadata"
        pressed={false}
        onPressedChange={fn()}
      />
    ),
  },
  decorators: Compact.decorators,
  play: async ({ canvasElement }) => {
    const bar = within(barOf(canvasElement))
    await expect(bar.getByText("AI · Metadata")).toBeVisible()
    await expect(bar.getByRole("button", { name: "Ask AI" })).toBeVisible()
  },
}

export const WithSidePanel: Story = {
  args: {
    asideOpen: true,
    aside: (
      <WorkspaceAside
        items={[
          {
            id: "inspector",
            label: "Inspector",
            icon: InformationCircleIcon,
            content: <p className="p-3 text-sm">Row 12 of invoices</p>,
          },
        ]}
        active="inspector"
        onActiveChange={fn()}
        onClose={fn()}
      />
    ),
  },
  play: async ({ canvas }) => {
    const toggle = canvas.getByRole("button", { name: "Toggle side panel" })
    await expect(toggle).toHaveAttribute("aria-pressed", "true")
    await expect(canvas.getByText("Row 12 of invoices")).toBeVisible()
  },
}

/**
 * In compact the side panel overlays the work area; Escape closes it —
 * including from the bar that opened it, which is where the focus sits after
 * the toggle or ⌘⌥B, and which is outside the overlaid column.
 */
export const CompactSidePanel: Story = {
  args: { ...WithSidePanel.args, compact: true },
  play: async ({ canvas, canvasElement }) => {
    const overlay = canvasElement.querySelector(
      '[data-slot="workspace-aside-overlay"]'
    )
    await expect(overlay).not.toHaveClass("hidden")
    // ⌘⌥B opens the column without moving the focus: Escape must reach it
    // from the work area, which is outside the column entirely.
    const scratch = canvas.getByLabelText("unpaid invoices.sql scratch")
    scratch.focus()
    await expect(scratch).toHaveFocus()
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(overlay).toHaveClass("hidden"))
  },
}

export const NoTab: Story = {
  parameters: { noTab: true },
  args: {
    children: (
      <p className="flex h-full items-center justify-center text-sm text-muted-foreground">
        No console open
      </p>
    ),
  },
}

/** A long right-to-left name truncates; the environment stays visible. */
export const HostileConnectionName: Story = {
  args: {
    connectionName:
      "مخزن الفواتير الرئيسي <img src=x onerror=alert(1)> ‮production-eu-west-1-replica",
    readOnly: true,
  },
  play: async ({ canvasElement }) => {
    const bar = within(barOf(canvasElement))
    await expect(bar.getByText(/<img src=x/)).toHaveAttribute("dir", "auto")
    await expect(
      bar.getByText(/production/i, {
        selector: "[data-slot=environment-badge]",
      })
    ).toBeVisible()
  },
}

export const Disconnect: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Disconnect" }))
    await expect(args.onDisconnect).toHaveBeenCalled()
  },
}

/**
 * The Data tab as `object-view.tsx` composes it: the preview bar, then the
 * result panel in a `min-h-0 flex-1` box.
 */
function ObjectDataTab() {
  const [tab, setTab] = React.useState<ObjectTab>("data")
  return (
    <ObjectViewFrame
      name="invoices"
      kind="table"
      tab={tab}
      onTabChange={setTab}
      dataUnavailable={null}
      panels={{
        data: (
          <>
            <PreviewControls
              canFilter
              canSort
              columns={invoiceColumns.map((column) => column.name)}
              applied={{ predicate: null, sort: [], offset: 0 }}
              status="loaded"
              pagination={null}
              onApplyPredicate={fn()}
              onApplySort={fn()}
              onPage={fn()}
              onCancel={fn()}
              onLoadColumns={fn()}
            />
            <div className="min-h-0 flex-1">
              <ResultPanel
                state={{
                  status: "populated",
                  result: "chain-preview",
                  columns: invoiceColumns,
                  rows: 7,
                  complete: true,
                  truncated: false,
                  cancelled: false,
                  elapsedMs: 9,
                }}
                fetchPage={syntheticPages(7, 0)}
                footerNote="Preview · total row count not requested"
              />
            </div>
          </>
        ),
        structure: <p className="p-3 text-sm">Structure</p>,
        indexes: null,
        constraints: null,
        relations: null,
        definition: null,
      }}
    />
  )
}

/** The panels `workspace-screen.tsx` renders, with its own class chain. */
function ChainPanels() {
  return (
    <>
      <TabsContent
        value={tabPanelValue("object:invoices")}
        keepMounted
        className="h-full min-h-0"
      >
        <ObjectDataTab />
      </TabsContent>
      <TabsContent
        value={tabPanelValue("console:1")}
        keepMounted
        className="h-full min-h-0"
      >
        {/* The console's own chain: a vertical resizable group between the
            editor and the result. */}
        <ConsoleView
          toolbar={<div className="h-10 shrink-0 border-b px-3">Console</div>}
          notice={null}
          draftNotice="Draft saved locally"
          editor={
            <div className="h-full bg-muted/30 p-3 text-sm">SELECT 1</div>
          }
          results={
            <ResultPanel
              state={{
                status: "populated",
                result: "chain-console",
                columns: invoiceColumns,
                rows: 7,
                complete: true,
                truncated: false,
                cancelled: false,
                elapsedMs: 9,
              }}
              fetchPage={syntheticPages(7, 0)}
            />
          }
        />
      </TabsContent>
    </>
  )
}

function ChainStory(args: React.ComponentProps<typeof WorkspaceLayout>) {
  const [active, setActive] = React.useState(args.activeTab)
  return (
    <WorkspaceLayout
      {...args}
      activeTab={active}
      onActiveTabChange={(key) => {
        setActive(key)
        args.onActiveTabChange(key)
      }}
      tabs={
        <WorkspaceTabs
          tabs={tabs}
          onClose={fn()}
          onNewConsole={fn()}
          onCancelOpening={fn()}
        />
      }
      statusBar={
        <StatusBar
          connectionName={args.connectionName}
          driver="postgres"
          environment={args.environment}
          readOnly={args.readOnly}
          execution={{ status: "done", rows: 7, elapsedMs: 9 }}
        />
      }
    >
      <ChainPanels />
    </WorkspaceLayout>
  )
}

/** Rows are only asked for the viewport: a grid of no height asks for none. */
async function expectRowsDrawn(
  grid: HTMLElement,
  canvas: ReturnType<typeof within>
) {
  await waitFor(() =>
    expect(grid.getBoundingClientRect().height).toBeGreaterThan(200)
  )
  await waitFor(() =>
    expect(canvas.getAllByRole("gridcell").length).toBeGreaterThan(0)
  )
}

/**
 * The height chain of the real screen, in a 900 × 600 window.
 *
 * `ResultGrid` asks the backend for the pages its virtualizer says are
 * visible. In a box of no height there are none, so nothing is fetched and no
 * row is drawn — while the footer still says « 7 rows shown · 9 ms ». Every
 * other grid story wraps the panel in a fixed-height box, which is exactly the
 * hole this one closes: here the height comes from the chain
 * `SidebarInset → Tabs → ResizablePanel → TabsContent.h-full → ObjectViewFrame
 * → TabsContent.flex-1 → div.min-h-0.flex-1 → ResultPanel → ResultGrid`.
 */
export const DataGridFillsItsPanel: Story = {
  args: { activeTab: "object:invoices" },
  decorators: [
    (Story) => (
      <div className="h-[600px] w-[900px] overflow-hidden border">
        <Story />
      </div>
    ),
  ],
  render: ChainStory,
  play: async ({ canvas }) => {
    await expectRowsDrawn(await canvas.findByRole("grid"), canvas)
  },
}

/**
 * A `keepMounted` panel that comes back to the front had no height while it
 * was hidden. The virtualizer has to measure again, or the grid stays empty
 * after a tab switch even when the chain itself is sound.
 */
export const DataGridRemeasuresOnTabReturn: Story = {
  args: { activeTab: "console:1" },
  decorators: [
    (Story) => (
      <div className="h-[600px] w-[900px] overflow-hidden border">
        <Story />
      </div>
    ),
  ],
  render: ChainStory,
  play: async ({ canvas, canvasElement }) => {
    // The console tab draws its own grid, under a vertical resizable group.
    await expectRowsDrawn(await canvas.findByRole("grid"), canvas)

    // The object tab is mounted behind it and measures nothing: this is the
    // state the virtualizer has to leave behind when the tab comes back.
    const hidden = Array.from(
      canvasElement.querySelectorAll('[role="grid"]')
    ).find((grid) => grid.getBoundingClientRect().height === 0)
    await expect(hidden).not.toBeUndefined()

    await userEvent.click(canvas.getByRole("tab", { name: /invoices$/ }))
    await expectRowsDrawn(await canvas.findByRole("grid"), canvas)
  },
}

function barOf(root: HTMLElement) {
  const bar = root.querySelector<HTMLElement>('[data-slot="workspace-bar"]')
  if (!bar) throw new Error("no workspace bar")
  return bar
}
