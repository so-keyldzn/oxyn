import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantSampleApproval } from "./assistant-sample-approval"
import type { SampleRequest } from "./assistant-sample-approval"
import { Button } from "@/components/ui/button"
import type { RelationField } from "@/lib/ipc/types"

function field(
  name: string,
  logicalType: string,
  rest: Partial<RelationField> = {}
): RelationField {
  return {
    name,
    position: 0,
    logicalType,
    rawType: logicalType,
    nullable: true,
    default: null,
    comment: null,
    primaryKey: false,
    ...rest,
  }
}

const REQUEST: SampleRequest = {
  id: "approval-1",
  source: "public.clients",
  rows: 12,
  destination: "Claude Sonnet",
  reach: "remote",
  fields: [
    field("id", "bigint", { nullable: false, primaryKey: true }),
    field("email", "text", { comment: "Login address, unique per account" }),
    field("signed_up_at", "timestamptz"),
    field("notes", "text"),
  ],
}

// The dialog portals out of the story root and animates in, so every story
// looks at `document.body` and waits for the frame where it is really there —
// the same shape as `approval-dialog.stories.tsx`.
const body = () => within(document.body)

/**
 * WCAG contrast between two computed colours.
 *
 * Axe checks text contrast; it does **not** check the contrast of a control's
 * boundary (WCAG 1.4.11). An unticked checkbox is nothing but its boundary, so
 * on a consent screen that opens with every box unticked, this is measured
 * here rather than trusted to a rule that does not exist.
 *
 * The theme is written in OKLCH, and so is what `getComputedStyle` returns: a
 * canvas paints each colour to read it back as sRGB, whatever its notation.
 */
function contrast(first: string, second: string) {
  const luminance = (color: string) => {
    const canvas = document.createElement("canvas")
    canvas.width = 1
    canvas.height = 1
    const context = canvas.getContext("2d", { willReadFrequently: true })
    if (!context) throw new Error("No 2D context")
    // A colour the canvas cannot read would keep the sentinel and measure a
    // wrong ratio in silence: the story fails loudly instead.
    context.fillStyle = "#010203"
    context.fillStyle = color
    if (context.fillStyle === "#010203" && color !== "#010203")
      throw new Error(`Unreadable colour: ${color}`)
    context.fillRect(0, 0, 1, 1)
    const [red = 0, green = 0, blue = 0] = context.getImageData(0, 0, 1, 1).data
    const [r, g, b] = [red, green, blue].map((channel) => {
      const value = channel / 255
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
    }) as [number, number, number]
    return 0.2126 * r + 0.7152 * g + 0.0722 * b
  }
  const [light, dark] = [luminance(first), luminance(second)].sort(
    (a, b) => b - a
  ) as [number, number]
  return (light + 0.05) / (dark + 0.05)
}

async function uncheckedBoxesAreVisible() {
  const screen = body()
  const [box] = await screen.findAllByRole("checkbox")
  const popup = box?.closest('[data-slot="alert-dialog-content"]')
  await expect(popup).not.toBeNull()
  await waitFor(() => {
    const ratio = contrast(
      getComputedStyle(box as HTMLElement).borderTopColor,
      getComputedStyle(popup as HTMLElement).backgroundColor
    )
    expect(ratio).toBeGreaterThanOrEqual(3)
  })
}

const meta = {
  title: "Oxyn/Assistant/SampleApproval",
  component: AssistantSampleApproval,
  args: {
    request: REQUEST,
    connectionName: "commerce-prod",
    environment: "production",
    tier: "sampled",
    deciding: false,
    onDecide: fn(),
  },
} satisfies Meta<typeof AssistantSampleApproval>

export default meta
type Story = StoryObj<typeof meta>

/**
 * As it opens: nothing ticked, and the action that sends is not reachable.
 *
 * A pre-ticked box would turn « explicitly approved » into « did not object »,
 * which is not what ADR-0006 says.
 */
export const NothingApprovedYet: Story = {
  play: async ({ args }) => {
    const screen = body()
    const boxes = await screen.findAllByRole("checkbox")
    for (const box of boxes) await expect(box).not.toBeChecked()
    const [line] = await screen.findAllByText(
      "No column ticked. Nothing would be sent."
    )
    await waitFor(() => expect(line).toBeVisible())
    const [send] = await screen.findAllByRole("button", {
      name: /Send 0 of 4 columns/,
    })
    await expect(send).toBeDisabled()
    await expect(args.onDecide).not.toHaveBeenCalled()
    // No « select all »: sending sixty columns of real data should cost sixty
    // gestures, and that friction is the feature.
    await expect(
      screen.queryByRole("button", { name: /Select all/i })
    ).toBeNull()
    // No « remember this choice » in any form: an approval covers one send.
    await expect(
      screen.queryByRole("checkbox", { name: /remember|always|don't ask/i })
    ).toBeNull()
    await expect(
      screen.queryByRole("switch", { name: /remember|always|don't ask/i })
    ).toBeNull()
  },
}

/** Cancel holds the focus, and Enter alone never sends. */
export const CancelIsFocusedAndEnterDoesNotSend: Story = {
  play: async ({ args }) => {
    const screen = body()
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.click(
      await screen.findByRole("checkbox", { name: /email/ })
    )
    // Enter pressed away from the buttons must not reach the action that sends.
    const list = await screen.findByRole("list", {
      name: "Columns that may be sent",
    })
    list.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onDecide).not.toHaveBeenCalled()
  },
}

/** Ticked one by one, and the action names what leaves and where. */
export const TwoColumnsApproved: Story = {
  play: async ({ args }) => {
    const screen = body()
    await userEvent.click(await screen.findByRole("checkbox", { name: /id/ }))
    await userEvent.click(
      await screen.findByRole("checkbox", { name: /email/ })
    )
    const [count] = await screen.findAllByText("2 of 4 columns ticked.")
    await waitFor(() => expect(count).toBeVisible())
    await userEvent.click(
      await screen.findByRole("button", {
        name: /Send 2 of 4 columns to Claude Sonnet · off this machine/,
      })
    )
    // The names, in catalog order — never the values, which this screen has no
    // prop to receive (I-03).
    await expect(args.onDecide).toHaveBeenCalledWith(["id", "email"])
  },
}

/**
 * An approval covers one send, and is never remembered for the source.
 *
 * The next send on the **same** table arrives without the screen closing in
 * between — the case an effect keyed on the source would have got wrong. It
 * starts unticked, and the action is inert again.
 */
export const NextSendOnTheSameSourceStartsUnticked: Story = {
  render: function Harness(args) {
    const [send, setSend] = React.useState(1)
    return (
      <>
        <AssistantSampleApproval
          {...args}
          request={{ ...REQUEST, id: `approval-${send}` }}
        />
        {/* Story harness, not part of the component: it stands in for the bus
            holding back a second send on the same source. It sits outside the
            dialog, so it is reached through the document. */}
        <Button size="sm" variant="outline" onClick={() => setSend(2)}>
          Next send
        </Button>
      </>
    )
  },
  play: async () => {
    const screen = body()
    const email = await screen.findByRole("checkbox", { name: /email/ })
    await userEvent.click(email)
    await waitFor(() => expect(email).toBeChecked())

    // The harness button sits behind the modal, which makes the page inert to
    // a pointer. `click()` stands for the bus pushing the next send, which
    // needs no gesture from the user anyway.
    const next = screen.getByText("Next send").closest("button")
    await expect(next).not.toBeNull()
    next?.click()

    await waitFor(async () => {
      for (const box of screen.getAllByRole("checkbox")) {
        await expect(box).not.toBeChecked()
      }
    })
    const [line] = await screen.findAllByText(
      "No column ticked. Nothing would be sent."
    )
    await waitFor(() => expect(line).toBeVisible())
  },
}

/** Closing without approving is a refusal, not « approved nothing ». */
export const Cancelled: Story = {
  play: async ({ args }) => {
    const screen = body()
    await userEvent.click(await screen.findByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(args.onDecide).toHaveBeenCalledWith(null))
  },
}

export const Sending: Story = {
  args: { deciding: true },
  play: async () => {
    const screen = body()
    const boxes = await screen.findAllByRole("checkbox")
    // Base UI keeps a disabled checkbox focusable and marks it `aria-disabled`
    // rather than `disabled`, so it stays discoverable to a screen reader.
    // `toBeDisabled()` would look for an attribute that is deliberately absent.
    for (const box of boxes) {
      await expect(box).toHaveAttribute("aria-disabled", "true")
    }
  },
}

/** The source reports no column: nothing to approve, and it is said. */
export const NoColumn: Story = {
  args: { request: { ...REQUEST, fields: [] } },
  play: async () => {
    const screen = body()
    const [line] = await screen.findAllByText(/This source reports no column/)
    await waitFor(() => expect(line).toBeVisible())
    await expect(screen.queryByRole("button", { name: /^Send/ })).toBeNull()
    await expect(screen.queryAllByRole("checkbox")).toHaveLength(0)
  },
}

/**
 * The screen refuses to be the hole.
 *
 * Rendered under a tier that never lets a row value leave, it draws a refusal
 * instead of checkboxes — a dialog that offered to send values under
 * `Metadata` would be a defect that ships without a sound.
 */
export const RefusedUnderMetadata: Story = {
  args: { tier: "metadata" },
  play: async () => {
    const screen = body()
    const [line] = await screen.findAllByText(/row values never leave/)
    await waitFor(() => expect(line).toBeVisible())
    await expect(screen.queryAllByRole("checkbox")).toHaveLength(0)
    await expect(screen.queryByRole("button", { name: /^Send/ })).toBeNull()
  },
}

export const RefusedUnderLocal: Story = {
  args: { tier: "local" },
  play: async () => {
    const screen = body()
    const [line] = await screen.findAllByText(/Nothing leaves this machine/)
    await waitFor(() => expect(line).toBeVisible())
    await expect(screen.queryAllByRole("checkbox")).toHaveLength(0)
  },
}

/** A local model still gets the sample, but nothing leaves the machine. */
export const ToALocalModel: Story = {
  args: {
    request: { ...REQUEST, destination: "Ollama · llama3", reach: "local" },
  },
  play: async () => {
    const screen = body()
    await userEvent.click(await screen.findByRole("checkbox", { name: /id/ }))
    const send = await screen.findByRole("button", { name: /on this machine/ })
    await waitFor(() => expect(send).toBeVisible())
  },
}

/** An address Oxyn could not resolve counts as remote, and is said as such. */
export const ToAnUnresolvedAddress: Story = {
  args: { request: { ...REQUEST, reach: "unresolved" } },
  play: async () => {
    const screen = body()
    await userEvent.click(await screen.findByRole("checkbox", { name: /id/ }))
    const send = await screen.findByRole("button", {
      name: /to an unresolved address/,
    })
    await waitFor(() => expect(send).toBeVisible())
  },
}

/**
 * Sixty columns, hostile and right-to-left names, in a narrow window.
 *
 * The list scrolls down; the dialog never scrolls sideways.
 */
export const ManyAndHostileColumns: Story = {
  args: {
    request: {
      ...REQUEST,
      source: 'public.users"; DROP TABLE audit; --',
      fields: [
        field('email"; DROP TABLE audit; --', "text"),
        field("البريد_الإلكتروني", "text", { comment: "عنوان العميل" }),
        field(
          "a_column_whose_name_is_long_enough_to_need_two_full_lines",
          "character varying(255)"
        ),
        ...Array.from({ length: 57 }, (_, index) =>
          field(`attribute_${index}`, "numeric(12,2)")
        ),
      ],
    },
  },
  globals: { viewport: { value: "mobile1" } },
  play: async () => {
    const screen = body()
    // A column name is server input: shown as text, never interpreted.
    const [hostile] = await screen.findAllByText('email"; DROP TABLE audit; --')
    await waitFor(() => expect(hostile).toBeVisible())
    const list = await screen.findByRole("list", {
      name: "Columns that may be sent",
    })
    await expect(list.scrollWidth).toBeLessThanOrEqual(list.clientWidth)
  },
}

/**
 * The unticked boxes can be seen, in the dark theme.
 *
 * `border-input` alone measured 1.59:1 here — a box that is barely drawn at the
 * moment consent is asked.
 */
export const UncheckedBoxesVisibleInDark: Story = {
  globals: { theme: "dark" },
  play: uncheckedBoxesAreVisible,
}

/**
 * The same, in the light theme — which Storybook does not render by default,
 * so axe had never checked this screen's text in it either.
 */
export const UncheckedBoxesVisibleInLight: Story = {
  globals: { theme: "light" },
  play: uncheckedBoxesAreVisible,
}
