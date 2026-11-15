import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"
import { CatalogSource } from "./catalog-source"

const meta = {
  title: "Oxyn/Catalog source",
  component: CatalogSource,
  args: { name: null, onReturn: fn() },
} satisfies Meta<typeof CatalogSource>
export default meta
type Story = StoryObj<typeof meta>
export const Connection: Story = {
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    await expect(c.queryByRole("combobox")).toBeNull()
    await expect(c.queryByRole("button")).toBeNull()
    await expect(c.queryByRole("status")).toBeNull()
  },
}
export const Console: Story = {
  args: { name: "Monthly report" },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.getByRole("status")).toHaveTextContent(
      "Browsing Monthly report"
    )
    const back = c.getByRole("button", {
      name: /Return to connection browsing/,
    })
    back.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onReturn).toHaveBeenCalledTimes(1)
    await expect(c.queryByRole("combobox")).toBeNull()
  },
}
export const Retired: Story = {
  args: {
    notice:
      "Console session no longer available; browsing the connection again.",
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("status")).toHaveTextContent(
      "browsing the connection again"
    )
  },
}
export const Collapsed: Story = {
  args: { name: "Monthly report" },
  decorators: [
    (Story) => (
      <div className="group w-12" data-collapsible="icon">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    const back = c.getByRole("button", {
      name: /currently browsing Monthly report/,
    })
    await expect(back).toBeVisible()
    back.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onReturn).toHaveBeenCalledTimes(1)
  },
}
