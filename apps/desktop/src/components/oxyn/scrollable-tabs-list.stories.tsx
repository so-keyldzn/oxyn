import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor } from "storybook/test"

import { ScrollableTabsList } from "./scrollable-tabs-list"
import { Tabs, TabsContent, TabsTrigger } from "@/components/ui/tabs"

const names = [
  "Data",
  "Structure",
  "Indexes",
  "Constraints",
  "Relations",
  "DDL",
]
const meta = {
  title: "Oxyn/ScrollableTabsList",
  component: ScrollableTabsList,
  args: { "aria-label": "Object views" },
  render: (args) => (
    <Tabs defaultValue="Data" className="w-72">
      <ScrollableTabsList {...args}>
        {names.map((name) => (
          <TabsTrigger key={name} value={name}>
            {name}
          </TabsTrigger>
        ))}
      </ScrollableTabsList>
      {names.map((name) => (
        <TabsContent key={name} value={name}>
          {name} panel
        </TabsContent>
      ))}
    </Tabs>
  ),
} satisfies Meta<typeof ScrollableTabsList>
export default meta
type Story = StoryObj<typeof meta>

async function assertGeometry(list: HTMLElement) {
  const viewport = list.closest<HTMLElement>(
    '[data-slot="scroll-area-viewport"]'
  )
  expect(viewport).not.toBeNull()
  if (!viewport) return
  await waitFor(() =>
    expect(viewport.scrollHeight).toBeLessThanOrEqual(viewport.clientHeight)
  )
  const active = list.querySelector<HTMLElement>("[data-active]")
  if (!active) throw new Error("No selected tab")
  const marker = getComputedStyle(active, "::after")
  const bottom =
    active.getBoundingClientRect().bottom - Number.parseFloat(marker.bottom)
  expect(Number.parseFloat(marker.opacity)).toBe(1)
  expect(bottom).toBeLessThanOrEqual(viewport.getBoundingClientRect().bottom)
}

export const WorkspaceHeight: Story = {
  play: async ({ canvas }) => {
    const list = canvas.getByRole("tablist", { name: "Object views" })
    await assertGeometry(list)
    canvas.getByRole("tab", { name: "Data" }).focus()
    await userEvent.keyboard("{End}")
    const last = canvas.getByRole("tab", { name: "DDL" })
    await waitFor(() => expect(last).toHaveFocus())
    const viewport = list.closest<HTMLElement>(
      '[data-slot="scroll-area-viewport"]'
    )
    if (!viewport) throw new Error("No scrolling viewport")
    await waitFor(() =>
      expect(last.getBoundingClientRect().right).toBeLessThanOrEqual(
        viewport.getBoundingClientRect().right + 1
      )
    )
    await assertGeometry(list)
  },
}
export const ObjectHeight: Story = {
  args: { size: "sm" },
  play: async ({ canvas }) => {
    await assertGeometry(canvas.getByRole("tablist"))
  },
}
