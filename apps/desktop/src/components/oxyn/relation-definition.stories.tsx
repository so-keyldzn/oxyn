import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { invoicesDefinition } from "./metadata-fixtures"
import { DefinitionActions, RelationDefinition } from "./relation-definition"

// Loading, empty and error states are the frame's: see `FacetFrame`.
const meta = {
  title: "Oxyn/RelationDefinition",
  component: RelationDefinition,
  decorators: [
    (Story) => (
      <div className="h-[420px] w-[900px] overflow-auto">
        <Story />
      </div>
    ),
  ],
  args: { definition: invoicesDefinition, stale: false },
} satisfies Meta<typeof RelationDefinition>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Reconstructed definition")).toBeVisible()
    await expect(canvas.getByText("Read only")).toBeVisible()
    await expect(canvas.getByLabelText("Object definition")).toHaveTextContent(
      "CREATE TABLE public.invoices"
    )
  },
}

export const Stale: Story = {
  args: { stale: true },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("This definition may be outdated.")
    ).toBeVisible()
  },
}

const onCopy = fn()
const onOpenInConsole = fn()

/** Opening the DDL in a console hands over text: nothing runs from here. */
export const Actions: Story = {
  render: (args) => (
    <div className="flex gap-2 p-2">
      <DefinitionActions
        definition={args.definition}
        onCopy={onCopy}
        onOpenInConsole={onOpenInConsole}
      />
    </div>
  ),
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: /Copy DDL/ }))
    await expect(onCopy).toHaveBeenCalledWith(invoicesDefinition.sql)
    await userEvent.click(
      canvas.getByRole("button", { name: /Open DDL in console/ })
    )
    await expect(onOpenInConsole).toHaveBeenCalledWith(invoicesDefinition.sql)
  },
}
