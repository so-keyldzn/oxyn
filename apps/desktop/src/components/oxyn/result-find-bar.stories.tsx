import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { ResultFindBar } from "./result-find-bar"

const meta = {
  title: "Oxyn/ResultFindBar",
  component: ResultFindBar,
  decorators: [
    (Story) => (
      <div className="w-[900px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    answer: null,
    searching: false,
    error: null,
    onFind: fn(),
    onClear: fn(),
  },
} satisfies Meta<typeof ResultFindBar>

export default meta
type Story = StoryObj<typeof meta>

export const Initial: Story = {
  play: async ({ canvas, args }) => {
    const field = canvas.getByLabelText("Find in loaded results")
    await userEvent.type(field, "Globex{Enter}")
    await expect(args.onFind).toHaveBeenCalledWith("Globex", "first")
    await userEvent.keyboard("{Enter}")
    await expect(args.onFind).toHaveBeenLastCalledWith("Globex", "next")
    await userEvent.keyboard("{Shift>}{Enter}{/Shift}")
    await expect(args.onFind).toHaveBeenLastCalledWith("Globex", "previous")
    await userEvent.keyboard("{Escape}")
    await expect(args.onClear).toHaveBeenCalled()
  },
}

export const NoMatch: Story = {
  args: {
    answer: {
      total: 0,
      row: null,
      ordinal: null,
      skippedBatches: 0,
      capped: false,
    },
  },
  play: async ({ canvas }) => {
    await userEvent.type(
      canvas.getByLabelText("Find in loaded results"),
      "zzz{Enter}"
    )
    await expect(canvas.getByRole("status")).toHaveTextContent("No match")
  },
}

/** What was not searched is said, or « no match » would lie. */
export const PartialSearch: Story = {
  args: {
    answer: {
      total: 50_000,
      row: 12,
      ordinal: 1,
      skippedBatches: 3,
      capped: true,
    },
  },
  play: async ({ canvas }) => {
    await userEvent.type(
      canvas.getByLabelText("Find in loaded results"),
      "1{Enter}"
    )
    const status = canvas.getByRole("status")
    await expect(status).toHaveTextContent(
      /or more · the search stopped counting/
    )
    await expect(status).toHaveTextContent(
      /3 batches not searched: they spilled to disk/
    )
  },
}

export const Expired: Story = {
  args: { error: "This result is no longer available." },
}

/** A needle in Arabic, in a narrow panel: the summary truncates, the input stays. */
export const NarrowRtl: Story = {
  args: {
    answer: {
      total: 1_284_512,
      row: 40,
      ordinal: 12,
      skippedBatches: 2,
      capped: true,
    },
  },
  decorators: [
    (Story) => (
      <div className="w-[420px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, args }) => {
    await userEvent.type(
      canvas.getByLabelText("Find in loaded results"),
      "شركة{Enter}"
    )
    await expect(args.onFind).toHaveBeenCalledWith("شركة", "first")
    await expect(canvas.getByRole("status")).toHaveTextContent(/not searched/)
  },
}

export const Searching: Story = { args: { searching: true } }
