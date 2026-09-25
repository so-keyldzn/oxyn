import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"
import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowRight01Icon } from "@hugeicons/core-free-icons"

import {
  ButtonItemActions,
  ButtonItemContent,
  ButtonItemDescription,
  ButtonItemMedia,
  ButtonItemTitle,
} from "./button-item"
import { DriverLogo } from "./driver-logo"
import { Badge } from "@/components/ui/badge"
import { Item } from "@/components/ui/item"

function ConnectionButton({ onOpen }: { onOpen: () => void }) {
  return (
    <Item
      variant="outline"
      render={<button type="button" onClick={onOpen} />}
      className="max-w-sm text-left"
    >
      <ButtonItemMedia>
        <DriverLogo driver="postgres" className="size-4" />
      </ButtonItemMedia>
      <ButtonItemContent className="min-w-0">
        <ButtonItemTitle>
          commerce-prod
          <Badge variant="outline">Production</Badge>
        </ButtonItemTitle>
        <ButtonItemDescription>
          PostgreSQL · db.internal:5432
        </ButtonItemDescription>
      </ButtonItemContent>
      <ButtonItemActions>
        <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} aria-hidden />
      </ButtonItemActions>
    </Item>
  )
}

const meta = {
  title: "Oxyn/ButtonItem",
  component: ConnectionButton,
  args: { onOpen: fn() },
  decorators: [
    (Story) => (
      <div className="p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConnectionButton>

export default meta
type Story = StoryObj<typeof meta>

/**
 * An `Item` rendered as a button, with media, title, description and actions:
 * every part is phrasing content, so no `div` nor `p` sits under the button —
 * the invalid HTML the generated `Item` parts would put there.
 */
export const WithEveryPart: Story = {
  play: async ({ args, canvasElement }) => {
    const canvas = within(canvasElement)
    const button = canvas.getByRole("button", { name: /commerce-prod/ })
    await expect(button.querySelector("div, p")).toBeNull()
    for (const slot of [
      "item-media",
      "item-content",
      "item-title",
      "item-description",
      "item-actions",
    ]) {
      await expect(button.querySelector(`[data-slot="${slot}"]`)).not.toBeNull()
    }
    // The whole row is one control, named by what it shows.
    await expect(button).toHaveAccessibleName(/PostgreSQL · db\.internal:5432/)
    await userEvent.click(button)
    await expect(args.onOpen).toHaveBeenCalledOnce()
  },
}

/** The same parts, legible on the dark theme. */
export const Dark: Story = {
  decorators: [
    (Story) => (
      <div className="dark bg-background p-3 text-foreground">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const button = within(canvasElement).getByRole("button")
    await expect(button.querySelector("div, p")).toBeNull()
  },
}
