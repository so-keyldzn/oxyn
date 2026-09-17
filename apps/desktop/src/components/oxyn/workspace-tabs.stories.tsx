import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { WorkspaceTabs } from "./workspace-tabs"
import type { WorkspaceTabItem } from "./workspace-tabs"
import { Tabs, TabsContent } from "@/components/ui/tabs"

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
    title: "console_1.sql",
    fromAgent: false,
    activity: "idle",
    unsaved: false,
  },
  {
    kind: "console",
    key: "console:2",
    title: "unpaid invoices.sql",
    fromAgent: false,
    activity: "running",
    unsaved: true,
  },
  {
    kind: "console",
    key: "console:3",
    title: "",
    fromAgent: true,
    activity: "approval",
    unsaved: true,
  },
]

const meta = {
  title: "Oxyn/WorkspaceTabs",
  component: WorkspaceTabs,
  args: {
    tabs,
    onClose: fn(),
    onNewConsole: fn(),
    onCancelOpening: fn(),
  },
  render: function Render(args) {
    const [active, setActive] = React.useState<string>(
      args.active ?? args.tabs[1]?.key ?? args.tabs[0]?.key ?? ""
    )
    return (
      <Tabs
        value={active}
        onValueChange={(value) => setActive(String(value))}
        className="w-[720px] max-w-full gap-0 p-3"
      >
        {/* The strip lives in the workspace's 48 px bar. */}
        <div className="flex h-12 items-center">
          <WorkspaceTabs {...args} active={active} />
        </div>
        {args.tabs.map((tab) => (
          <TabsContent key={tab.key} value={tab.key} className="p-3">
            Panel of {tab.title || "Untitled query"}
          </TabsContent>
        ))}
      </Tabs>
    )
  },
} satisfies Meta<typeof WorkspaceTabs>

export default meta
type Story = StoryObj<typeof meta>

/** Arrow keys move between tabs; Delete closes the focused one. */
export const Keyboard: Story = {
  play: async ({ canvas, args }) => {
    const first = canvas.getByRole("tab", { name: /console_1\.sql/ })
    await expect(first).toHaveAttribute("aria-selected", "true")
    first.focus()
    await userEvent.keyboard("{ArrowRight}")
    const second = canvas.getByRole("tab", { name: /unpaid invoices\.sql/ })
    await waitFor(() => expect(second).toHaveFocus())
    await userEvent.keyboard("{Delete}")
    await expect(args.onClose).toHaveBeenCalledWith("console:2")
    await expect(canvas.getByText("AI · Untitled query")).toBeVisible()
  },
}

export const OneConsole: Story = {
  args: { tabs: tabs.slice(1, 2) },
  /**
   * The only mark of the active tab is the line `variant="line"` draws five
   * pixels *below* the trigger. The strip scrolls, so a box too short for it
   * clips it away and nothing at all says which tab is shown.
   */
  play: async ({ canvas }) => {
    const list = canvas.getByRole("tablist")
    const active = canvas.getByRole("tab", { selected: true })
    const strip = list.getBoundingClientRect()
    await expect(active.getBoundingClientRect().bottom + 5 + 2).toBeLessThan(
      strip.bottom
    )
    // And the strip scrolls sideways only: a taller trigger than its box
    // would let the row slide up and down under the pointer.
    await expect(list.scrollHeight).toBeLessThanOrEqual(list.clientHeight)
  },
}

const many: Array<WorkspaceTabItem> = Array.from(
  { length: 12 },
  (_, index) => ({
    kind: "console" as const,
    key: `console:${index + 1}`,
    title:
      index === 4
        ? "تقرير الفواتير غير المدفوعة — un nom très long qui ne tient pas.sql"
        : `console_${index + 1}.sql`,
    fromAgent: false,
    activity: "idle" as const,
    unsaved: index % 3 === 0,
  })
)

const narrow = (Story: () => React.ReactElement) => (
  <div className="w-[420px]">
    <Story />
  </div>
)

/** Twelve tabs in a narrow window scroll instead of wrapping or overlapping. */
export const Overflow: Story = {
  args: { tabs: many },
  decorators: [narrow],
}

/**
 * ⌘T with a full strip activates a tab far to the right. It is brought back
 * into sight: a console opened where nothing seems to have happened reads as
 * a shortcut that did nothing.
 */
export const ActiveTabIsBroughtIntoView: Story = {
  args: { tabs: many, active: "console:12" },
  decorators: [narrow],
  play: async ({ canvas }) => {
    const list = canvas.getByRole("tablist")
    const active = canvas.getByRole("tab", { selected: true })
    await expect(active).toHaveAccessibleName(/console_12\.sql/)
    await waitFor(() => {
      const strip = list.getBoundingClientRect()
      const tab = active.getBoundingClientRect()
      expect(tab.right).toBeLessThanOrEqual(strip.right + 1)
      expect(tab.left).toBeGreaterThanOrEqual(strip.left - 1)
    })
  },
}

export const OpeningAConsole: Story = {
  args: { opening: true },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.queryByRole("button", { name: "New console" })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: /Cancel new console/ })
    )
    await expect(args.onCancelOpening).toHaveBeenCalled()
  },
}

export const HostileName: Story = {
  args: {
    tabs: [
      {
        kind: "object",
        key: "object:hostile",
        title: '"users"; DROP TABLE audit; --<img src=x onerror=alert(1)>',
        objectKind: "view",
      },
    ],
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("tab", { name: /DROP TABLE audit/ })
    ).toBeVisible()
    await expect(canvas.queryByRole("img", { name: "x" })).toBeNull()
  },
}

/**
 * Inactive tabs in the light theme. The generated trigger drew them at
 * 4.27:1 — under AA — and every other story runs axe in dark only, so nothing
 * failed. axe runs here against the light palette.
 */
export const Light: Story = { globals: { theme: "light" } }
