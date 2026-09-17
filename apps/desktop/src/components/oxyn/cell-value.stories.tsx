import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { CellValue, cellText, formatBytes } from "./cell-value"

const meta = {
  title: "Oxyn/CellValue",
  component: CellValue,
  decorators: [
    (Story) => (
      // The grid draws a cell in a narrow, fixed-width box: the same
      // constraint is what makes truncation and the size marker visible.
      <div className="flex w-56 items-center border px-2 py-1 font-mono text-reading">
        <Story />
      </div>
    ),
  ],
  args: { cell: "Acme SA" },
} satisfies Meta<typeof CellValue>

export default meta
type Story = StoryObj<typeof meta>

export const Text: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Acme SA")).toBeVisible()
    await expect(cellText("Acme SA")).toBe("Acme SA")
  },
}

/**
 * An absent value is its own token. A text column may literally hold the word
 * « NULL », so the two must never be drawn the same way.
 */
export const AbsentIsNotTheWordNull: Story = {
  args: { cell: null },
  play: async ({ canvasElement }) => {
    const absent = canvasElement.querySelector("[data-null]")
    await expect(absent).not.toBeNull()
    await expect(absent).toHaveTextContent("∅ NULL")
    // What is copied for an absent value is empty, not the marker.
    await expect(cellText(null)).toBe("")
  },
}

/** The literal text « NULL » is a value, drawn as text and not as absence. */
export const TheWordNull: Story = {
  args: { cell: "NULL" },
  play: async ({ canvas, canvasElement }) => {
    await expect(canvas.getByText("NULL")).toBeVisible()
    await expect(canvasElement.querySelector("[data-null]")).toBeNull()
  },
}

/**
 * A value Rust cut keeps its text and says how large it really is: the grid
 * never pretends a prefix is the whole value.
 */
export const Truncated: Story = {
  args: {
    cell: {
      text: '{"customer":{"id":42,"tags":["a","b"],"history":[{"at":"2026-01-01"',
      fullBytes: 3_145_728,
    },
  },
  play: async ({ canvas, canvasElement }) => {
    const truncated = canvasElement.querySelector("[data-truncated]")
    await expect(truncated).toHaveAttribute(
      "title",
      "Truncated · 3.0 MB in full"
    )
    await expect(canvas.getByText("3.0 MB")).toBeVisible()
  },
}

/** A type the formatter cannot render is said, never shown as an empty cell. */
export const Unrenderable: Story = {
  args: { cell: { unrenderable: "unsupported type: Dictionary" } },
  play: async ({ canvas, canvasElement }) => {
    await expect(canvas.getByText("⟨unrenderable⟩")).toBeVisible()
    await expect(
      canvasElement.querySelector("[data-unrenderable]")
    ).toHaveAttribute("title", "unsupported type: Dictionary")
    // Nothing is copied for a value that could not be formatted.
    await expect(cellText({ unrenderable: "x" })).toBe("")
  },
}

/**
 * A cell is text, whatever the server sent. A value shaped like markup is
 * drawn as characters: this is the first place an XSS in the webview would
 * come from.
 */
export const HostileValueIsText: Story = {
  args: { cell: '<img src=x onerror="alert(1)">' },
  play: async ({ canvas, canvasElement }) => {
    await expect(
      canvas.getByText('<img src=x onerror="alert(1)">')
    ).toBeVisible()
    await expect(canvasElement.querySelector("img")).toBeNull()
  },
}

/** Sizes read as a professional expects them: bytes, then one decimal. */
export const ByteSizes: Story = {
  args: { cell: "sizes" },
  play: async () => {
    await expect(formatBytes(512)).toBe("512 B")
    await expect(formatBytes(2048)).toBe("2.0 KB")
    await expect(formatBytes(52_428_800)).toBe("50.0 MB")
  },
}
