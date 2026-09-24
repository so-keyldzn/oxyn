import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { ERD_MAX_TABLES, ErdDiagram } from "./erd-diagram"
import { hubAndSpokes, shopLinks, shopTables } from "./erd-fixtures"

const meta = {
  title: "Oxyn/Assistant/ERD diagram",
  component: ErdDiagram,
  args: { tables: shopTables, links: shopLinks, onOpenObject: fn() },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <div className="overflow-hidden rounded-lg border">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof ErdDiagram>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    // A hostile name is text: the button carries it, nothing ran it.
    const hostile = await canvas.findByRole("button", {
      name: 'Open public."users"; DROP TABLE audit; --',
    })
    await expect(hostile).toBeVisible()
    await userEvent.click(
      await canvas.findByRole("button", { name: "Open public.orders" })
    )
    await expect(args.onOpenObject).toHaveBeenCalledWith({
      catalog: null,
      namespace: "public",
      relation: "orders",
    })
    // The keys are written for a screen reader, not only drawn.
    await expect(
      canvas.getByText(
        "public.order_items (order_id) references public.orders (id)"
      )
    ).toBeInTheDocument()
    await expect(canvas.getByText("5 tables, 4 keys")).toBeVisible()
    // Wide tables show their keys first, and count the rest.
    await expect(canvas.getByText("5 more columns")).toBeInTheDocument()
  },
}

export const Keyboard: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const region = canvas.getByRole("region", { name: "Diagram of 5 tables" })
    region.focus()
    const viewport = canvasElement.querySelector<HTMLElement>(
      ".react-flow__viewport"
    )
    const before = viewport?.style.transform
    await userEvent.keyboard("{ArrowLeft}")
    await expect(viewport?.style.transform).not.toBe(before)
    await userEvent.click(canvas.getByRole("button", { name: "Zoom in" }))
    await userEvent.click(canvas.getByRole("button", { name: "Fit to view" }))
  },
}

export const TooManyTables: Story = {
  args: (() => {
    const { tables, links } = hubAndSpokes(ERD_MAX_TABLES - 1)
    return { tables, links, omitted: 12 }
  })(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(
        `· 12 more related tables not drawn: a diagram stops at ${ERD_MAX_TABLES}`
      )
    ).toBeVisible()
  },
}

export const WithoutOpening: Story = {
  args: { onOpenObject: undefined },
  play: async ({ canvasElement }) => {
    // Nothing to open with: no control that does nothing.
    const canvas = within(canvasElement)
    await expect(canvas.queryByRole("button", { name: /^Open / })).toBeNull()
  },
}

/**
 * Storybook renders dark by default: without this story axe would never check
 * the diagram's contrast in light.
 */
export const Light: Story = {
  globals: { theme: "light" },
}
