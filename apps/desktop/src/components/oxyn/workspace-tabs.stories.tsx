import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { WorkspaceTabs } from "./workspace-tabs"
import type { TabMenuTarget, WorkspaceTabItem } from "./workspace-tabs"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useActionSource } from "@/lib/actions/context"
import type { WorkspaceActions } from "@/lib/actions/context"

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

/**
 * A result being exported does not close: closing would cancel the write the
 * user started. The tab says why, and offers no cross.
 */
export const ResultExporting: Story = {
  args: {
    tabs: [
      ...tabs.slice(0, 2),
      {
        kind: "result",
        key: "result:r1",
        title: "Result #42",
        exporting: true,
      },
    ],
    active: "result:r1",
  },
  play: async ({ canvas, args }) => {
    const result = canvas.getByRole("tab", { name: /Result #42/ })
    await expect(result).toHaveTextContent("exporting")
    await expect(result).toHaveAttribute(
      "title",
      expect.stringMatching(/finish or cancel the export/)
    )
    await expect(result.querySelector('[data-slot="tab-close"]')).toBeNull()
    result.focus()
    await userEvent.keyboard("{Delete}")
    await expect(args.onClose).not.toHaveBeenCalled()
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

const nothing = () => undefined
const storyWorkspace: WorkspaceActions = {
  openConsole: nothing,
  closeActiveTab: nothing,
  nextTab: nothing,
  previousTab: nothing,
  showCatalog: nothing,
  showLibrary: nothing,
  focusPreview: nothing,
  focusConsole: nothing,
  toggleSidebar: nothing,
  toggleAside: nothing,
  openAssistant: nothing,
  switchConnection: nothing,
}

/** The workspace as the registry sees it: `Close` needs one open. */
function InWorkspace({ children }: { children: React.ReactNode }) {
  useActionSource(
    "workspace",
    {
      activeTab: "console:1",
      tabCount: tabs.length,
      consoleCount: 3,
      objectActive: false,
      hasAside: false,
      hasAssistant: false,
    },
    storyWorkspace
  )
  return children
}

const close = fn()
const closeOthers = fn()
const closeRight = fn()

/** The target each tab gives its menu, as the workspace screen builds it. */
function menuFor(key: string): TabMenuTarget {
  const index = tabs.findIndex((tab) => tab.key === key)
  return {
    state: {
      console: key.startsWith("console:"),
      count: tabs.length,
      toTheRight: tabs.length - index - 1,
      saved: key === "console:2",
    },
    actions: {
      close: () => close(key),
      closeOthers: () => closeOthers(key),
      closeRight: () => closeRight(key),
      closeAll: fn(),
      duplicate: fn(),
      rename: fn(),
      revealInLibrary: key.startsWith("console:") ? fn() : undefined,
    },
  }
}

async function openMenuOn(tab: HTMLElement) {
  await userEvent.pointer({ keys: "[MouseRight]", target: tab })
  const page = within(document.body)
  await page.findByRole("menu")
  return page
}

async function closeMenu(page: ReturnType<typeof within>) {
  await userEvent.keyboard("{Escape}")
  await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
}

/**
 * The menu of the tab under the pointer: on the last one, `Close to the
 * right` says why it cannot; `Close others` closes the others of **this**
 * tab; an object is neither duplicated nor renamed, and says so.
 */
export const TabMenu: Story = {
  args: { menuFor },
  decorators: [
    (Story) => (
      <InWorkspace>
        <Story />
      </InWorkspace>
    ),
  ],
  play: async ({ canvas }) => {
    const last = canvas.getByRole("tab", { name: /AI · Untitled query/ })
    let page = await openMenuOn(last)
    const right = page.getByRole("menuitem", { name: /Close to the right/ })
    await expect(right).toHaveAttribute("aria-disabled", "true")
    await expect(right).toHaveTextContent("No tab is to the right")
    // Not yet possible, and said so rather than hidden.
    await expect(
      page.getByRole("menuitem", { name: /Open in new window/ })
    ).toHaveTextContent("Oxyn opens a single window")
    await userEvent.click(page.getByRole("menuitem", { name: "Close others" }))
    await expect(closeOthers).toHaveBeenCalledWith("console:3")
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    page = await openMenuOn(canvas.getByRole("tab", { name: /^invoices/ }))
    await expect(
      page.getByRole("menuitem", { name: /Duplicate/ })
    ).toHaveTextContent("Only a console is duplicated")
    await expect(
      page.queryByRole("menuitem", { name: /Reveal in library/ })
    ).toBeNull()
    await userEvent.click(
      page.getByRole("menuitem", {
        name: /^Close(?! (others|to the right|all))/,
      })
    )
    await expect(close).toHaveBeenCalledWith("object:invoices")
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())

    page = await openMenuOn(canvas.getByRole("tab", { name: /console_1/ }))
    await expect(
      page.getByRole("menuitem", { name: /Reveal in library/ })
    ).toHaveTextContent("This console is not saved in the library")
    await closeMenu(page)
  },
}

/** The middle button closes the tab under the pointer, not the active one. */
export const MiddleClickCloses: Story = {
  play: async ({ canvas, args }) => {
    const tab = canvas.getByRole("tab", { name: /^invoices/ })
    tab.dispatchEvent(
      new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true })
    )
    await expect(args.onClose).toHaveBeenCalledWith("object:invoices")
  },
}

/**
 * Inactive tabs in the light theme. The generated trigger drew them at
 * 4.27:1 — under AA — and every other story runs axe in dark only, so nothing
 * failed. axe runs here against the light palette.
 */
export const Light: Story = { globals: { theme: "light" } }
