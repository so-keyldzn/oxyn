import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantQuestion } from "./assistant-question"
import { measureChips } from "./assistant-mention-measure"
import type { MentionView } from "@/lib/ipc/ai"

const meta = {
  title: "Oxyn/Assistant/Question",
  component: AssistantQuestion,
  args: {
    text: "How many active clients?",
    versions: { position: 1, count: 1, previous: null, next: null },
    busy: false,
    onEdit: fn(() => true),
    onSelectVersion: fn(),
    onCopy: fn(() => true),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantQuestion>

export default meta
type Story = StoryObj<typeof meta>

export const Asked: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.queryByRole("group", { name: "Versions of this exchange" })
    ).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Copy question" }))
    await expect(args.onCopy).toHaveBeenCalledWith("How many active clients?")
  },
}

export const Edited: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Edit question" }))
    const field = canvas.getByRole("textbox", { name: "Edit your question" })
    await waitFor(() => expect(field).toHaveFocus())
    await userEvent.clear(field)
    await userEvent.type(field, "How many active clients last month?{Enter}")
    // Sent as another version: what followed the first one stays with it.
    await expect(args.onEdit).toHaveBeenCalledWith(
      "How many active clients last month?"
    )
  },
}

export const EditCancelledWithEscape: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Edit question" }))
    const field = canvas.getByRole("textbox", { name: "Edit your question" })
    await userEvent.type(field, " and last month{Escape}")
    await expect(args.onEdit).not.toHaveBeenCalled()
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "Edit question" })
      ).toHaveFocus()
    )
  },
}

export const SeveralVersions: Story = {
  args: { versions: { position: 2, count: 3, previous: 4, next: 9 } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("2 / 3")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Previous version" })
    )
    await expect(args.onSelectVersion).toHaveBeenCalledWith(4)
  },
}

export const WhileAnAnswerRuns: Story = {
  args: {
    busy: true,
    versions: { position: 2, count: 2, previous: 1, next: null },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Editing mid-answer would fork the conversation in the middle of it.
    await expect(
      canvas.getByRole("button", { name: "Edit question" })
    ).toBeDisabled()
    await expect(
      canvas.getByRole("button", { name: "Previous version" })
    ).toBeDisabled()
  },
}

export const RefusedEditKeepsTheEditorOpen: Story = {
  args: { onEdit: fn(() => false) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Edit question" }))
    const field = canvas.getByRole("textbox", { name: "Edit your question" })
    await userEvent.type(field, " today{Enter}")
    await expect(field).toBeVisible()
  },
}

export const LongAndRightToLeft: Story = {
  args: {
    text: "كم عدد العملاء النشطين في قاعدة البيانات هذه، وما هي الجداول التي يجب النظر إليها؟",
  },
}

const ORDERS_VIEW: MentionView = {
  kind: "table",
  label: "orders",
  mention: {
    kind: "relation",
    address: { catalog: null, namespace: "public", relation: "orders" },
    field: null,
  },
  missing: false,
}

/**
 * The question shows its mentions as the composer did — the same chip — and
 * a chip opens its object. Nothing is guessed from the text: `@home` stays
 * text, since the question does not carry it.
 */
export const WithMentions: Story = {
  args: {
    text: "Mail @home the totals of @orders by @customers.country",
    mentions: [
      ORDERS_VIEW,
      {
        kind: "column",
        label: "customers.country",
        mention: {
          kind: "relation",
          address: {
            catalog: null,
            namespace: "public",
            relation: "customers",
          },
          field: "country",
        },
        missing: false,
      },
    ],
    onOpenObject: fn(),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const chips = canvasElement.querySelectorAll(
      "[data-slot=assistant-mention]"
    )
    await expect(Array.from(chips, (chip) => chip.textContent)).toEqual([
      "@orders",
      "@customers.country",
    ])
    await expect(canvas.getByText(/Mail @home the totals of/)).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Open column customers.country" })
    )
    await expect(args.onOpenObject).toHaveBeenCalledWith({
      catalog: null,
      namespace: "public",
      relation: "customers",
    })
  },
}

/** An object dropped since: dimmed, said, and it opens nothing. */
export const WithAMissingMention: Story = {
  args: {
    text: "What was in @archive_2019?",
    mentions: [
      {
        ...ORDERS_VIEW,
        label: "archive_2019",
        mention: {
          kind: "relation",
          address: {
            catalog: null,
            namespace: "public",
            relation: "archive_2019",
          },
          field: null,
        },
        missing: true,
      },
    ],
    onOpenObject: fn(),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· not found")).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: /Open table archive_2019/ })
    ).toBeNull()
  },
}

/** A name the server chose is text: markup in it is never rendered. */
export const WithAHostileName: Story = {
  args: {
    text: 'Explain @<img src=x onerror="alert(1)">',
    mentions: [
      {
        ...ORDERS_VIEW,
        label: '<img src=x onerror="alert(1)">',
      },
    ],
    onOpenObject: fn(),
  },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector("img")).toBeNull()
    const chip = canvasElement.querySelector("[data-slot=assistant-mention]")
    await expect(chip?.textContent).toBe('@<img src=x onerror="alert(1)">')
  },
}

function relation(
  kind: MentionView["kind"],
  label: string,
  missing = false
): MentionView {
  const [table = label, field = null] = label.split(".")
  return {
    kind,
    label,
    mention: {
      kind: "relation",
      address: { catalog: null, namespace: "public", relation: table },
      field,
    },
    missing,
  }
}

/**
 * In the bubble as in the field, a chip reads as a word: same centre as the
 * text beside it (1 px), and no line taller for holding one — on several
 * lines, a missing chip and a button chip included.
 */
export const MentionAlignment: Story = {
  args: {
    text: "Count @orders per month for the customers who ordered twice, then join @order_lines.order_id and @archive_2019 please",
    mentions: [
      relation("table", "orders"),
      relation("column", "order_lines.order_id"),
      relation("table", "archive_2019", true),
    ],
    onOpenObject: fn(),
  },
  decorators: [
    (Story) => (
      <div className="w-[320px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const bubble = canvasElement.querySelector<HTMLElement>(
      "[data-slot=bubble-content]"
    )
    if (bubble === null) throw new Error("the question's bubble")
    const measured = await measureChips(bubble)
    await expect(measured.chips).toBe(3)
    await expect(measured.alone).toBe(0)
    for (const offset of measured.offsets)
      await expect(offset).toBeLessThanOrEqual(1)
    await expect(measured.withChips).toBeGreaterThan(40)
    await expect(
      Math.abs(measured.withChips - measured.withoutChips)
    ).toBeLessThanOrEqual(0.5)
  },
}

export const MentionAlignmentLight: Story = {
  ...MentionAlignment,
  globals: { theme: "light" },
}
