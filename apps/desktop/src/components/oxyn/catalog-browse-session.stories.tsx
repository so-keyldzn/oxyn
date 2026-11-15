import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"
import { WorkspaceTabs } from "./workspace-tabs"
import { CatalogSource } from "./catalog-source"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useCatalogBrowsing } from "@/features/workspace/use-catalog-browsing"
import type { OpenConnection } from "@/lib/ipc/types"

const close = fn()
const run = fn()
const consoles = ["A", "B"].map((name) => ({
  key: `console:${name}`,
  title: `Draft ${name}`,
  session: {
    session: `session-${name}`,
    capabilities: ["TABLES"],
    readOnly: false,
    transactionState: "open" as const,
  },
}))
const open = {
  connection: "synthetic-source",
  session: "catalog-owner",
  capabilities: ["TABLES"],
} as OpenConnection
const meta = {
  title: "Oxyn/Catalog browsing from console",
  component: CatalogSource,
  args: { name: null, onReturn: fn() },
  parameters: { layout: "padded" },
  render: () => {
    const [active, setActive] = React.useState("console:B")
    const browsing = useCatalogBrowsing(open, consoles, (owner) =>
      owner.capabilities.includes("TABLES")
    )
    return (
      <Tabs value={active} onValueChange={(value) => setActive(String(value))}>
        <WorkspaceTabs
          active={active}
          tabs={consoles.map((entry) => ({
            key: entry.key,
            title: entry.title,
            kind: "console",
            fromAgent: false,
            activity: "idle",
            unsaved: true,
          }))}
          onClose={close}
          onNewConsole={run}
          onCancelOpening={run}
          menuFor={(key) => ({
            state: {
              console: true,
              count: 2,
              toTheRight: 0,
              saved: false,
              moveBlocked: null,
              browseBlocked: browsing.reason(key),
            },
            actions: {
              browseSession: () => {
                browsing.browse(key)
              },
            },
          })}
        />
        <CatalogSource
          name={browsing.name}
          notice={browsing.notice}
          onReturn={browsing.returnToConnection}
        />
        <TabsContent value="console:A">Draft A remains unchanged</TabsContent>
        <TabsContent value="console:B">Draft B remains unchanged</TabsContent>
      </Tabs>
    )
  },
} satisfies Meta<typeof CatalogSource>
export default meta
type Story = StoryObj<typeof meta>
export const BrowseWithoutActivatingAnotherConsole: Story = {
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    const other = c.getByRole("tab", { name: /Draft A/ })
    const active = c.getByRole("tab", { name: /Draft B/ })
    await expect(active).toHaveAttribute("aria-selected", "true")
    await userEvent.pointer({ keys: "[MouseRight]", target: other })
    const page = within(document.body)
    await userEvent.click(
      await page.findByRole("menuitem", { name: "Browse this console session" })
    )
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
    await expect(
      c.getByRole("status", { name: "Explorer browsing source" })
    ).toHaveTextContent("Browsing Draft A")
    await expect(active).toHaveAttribute("aria-selected", "true")
    await expect(c.getByText("Draft B remains unchanged")).toBeVisible()
    const back = c.getByRole("button", {
      name: /Return to connection browsing/,
    })
    back.focus()
    await userEvent.keyboard("{Enter}")
    await expect(
      c.queryByRole("status", { name: "Explorer browsing source" })
    ).toBeNull()
    await expect(active).toHaveAttribute("aria-selected", "true")
    await expect(close).not.toHaveBeenCalled()
    await expect(run).not.toHaveBeenCalled()
  },
}
export const BrowseFromKeyboardContextMenu: Story = {
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    c.getByRole("tab", { name: /Draft B/ }).focus()
    await userEvent.keyboard("{Shift>}{F10}{/Shift}")
    const page = within(document.body)
    const item = await page.findByRole("menuitem", {
      name: "Browse this console session",
    })
    item.focus()
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
    await expect(
      c.getByRole("status", { name: "Explorer browsing source" })
    ).toHaveTextContent("Browsing Draft B")
    await expect(close).not.toHaveBeenCalled()
    await expect(run).not.toHaveBeenCalled()
  },
}
