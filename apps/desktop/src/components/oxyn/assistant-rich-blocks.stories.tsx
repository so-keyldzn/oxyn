import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantMarkdown } from "./assistant-markdown"

const meta = {
  title: "Oxyn/Assistant/Rich blocks",
  component: AssistantMarkdown,
  args: { text: "", onOpenSql: fn(), onCopy: fn(() => true) },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantMarkdown>

export default meta
type Story = StoryObj<typeof meta>

const sql = [
  "```sql",
  "-- unpaid invoices <b>not HTML</b>",
  "SELECT customer, sum(amount) AS due",
  "FROM invoices",
  "WHERE paid_at IS NULL AND amount > 100",
  "GROUP BY customer;",
  "```",
].join("\n")

/** A closed block is coloured, as React text: the tags in it stay text. */
export const Highlighted: Story = {
  args: {
    text: `${sql}\n\n\`\`\`json\n{ "limit": 20, "strict": true }\n\`\`\``,
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const code = canvas.getByLabelText("Proposed SQL")
    // Shiki and its grammar load on demand: on a cold module cache the first
    // colouring takes seconds, well past `waitFor`'s default second.
    await waitFor(
      () => expect(code).toHaveAttribute("data-highlighted", "true"),
      { timeout: 15_000 }
    )
    await expect(code.querySelector("b")).toBeNull()
    await expect(code).toHaveTextContent("<b>not HTML</b>")
    // Colouring changes nothing of what the buttons carry.
    await userEvent.click(
      canvas.getByRole("button", { name: "Open in console" })
    )
    await expect(args.onOpenSql).toHaveBeenCalledWith(
      sql.split("\n").slice(1, -1).join("\n")
    )
    await expect(
      canvas.getAllByRole("button", { name: "Copy code" })
    ).toHaveLength(2)
  },
}

/** While the answer streams, the block is plain text: nothing is coloured. */
export const StreamingBlock: Story = {
  args: { text: "Here it is:\n\n```sql\nSELECT customer\nFROM invoi" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const code = canvas.getByLabelText("Code")
    await expect(code).not.toHaveAttribute("data-highlighted")
    await expect(code.querySelectorAll("span")).toHaveLength(0)
  },
}

export const HighlightedLight: Story = {
  ...Highlighted,
  globals: { theme: "light" },
}

const flowchart = [
  "```mermaid",
  "flowchart LR",
  "  orders -->|customer_id| customers",
  "  order_items -->|order_id| orders",
  '  x["<img src=x onerror=alert(1)>"] --> orders',
  "```",
].join("\n")

/** Drawn as an image: the source never becomes markup in the page. */
export const MermaidDiagram: Story = {
  args: { text: flowchart },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const image = await canvas.findByRole(
      "img",
      { name: "Diagram drawn from the mermaid source of this answer" },
      { timeout: 15_000 }
    )
    await expect(image.getAttribute("src")).toMatch(
      /^data:image\/svg\+xml;base64,/
    )
    // Nothing of the diagram lives in the document: not its SVG, not its tag.
    await expect(
      canvasElement.querySelector("svg[id^=oxyn-mermaid]")
    ).toBeNull()
    await expect(document.querySelector("img[src=x]")).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Show source" }))
    await expect(canvas.getByLabelText("Diagram source")).toHaveTextContent(
      "order_items -->|order_id| orders"
    )
  },
}

export const MermaidDiagramLight: Story = {
  ...MermaidDiagram,
  globals: { theme: "light" },
}

/** A diagram that does not parse: the error, readable, and its source. */
export const MermaidInvalid: Story = {
  args: { text: "```mermaid\nflowchart LR\n  A -->\n  ((( B\n```" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByText(
        "This diagram could not be drawn.",
        {},
        { timeout: 15_000 }
      )
    ).toBeVisible()
    await expect(canvas.getByLabelText("Diagram source")).toHaveTextContent(
      "((( B"
    )
  },
}

/** A diagram that tries to configure its own rendering is drawn without it. */
export const MermaidWithDirective: Story = {
  args: {
    text: [
      "```mermaid",
      '%%{init: {"securityLevel": "loose", "htmlLabels": true}}%%',
      "sequenceDiagram",
      "  Oxyn->>Server: SELECT 1",
      "  Server-->>Oxyn: 1 row",
      "```",
    ].join("\n"),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByText(
        /configuration lines .* were ignored/,
        {},
        {
          timeout: 15_000,
        }
      )
    ).toBeVisible()
    await expect(
      canvas.getByRole("img", {
        name: "Diagram drawn from the mermaid source of this answer",
      })
    ).toBeVisible()
  },
}

/** Still streaming: the block is text, and mermaid is not even loaded. */
export const MermaidStreaming: Story = {
  args: { text: "```mermaid\nflowchart LR\n  A --> B" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByLabelText("Code")).toBeVisible()
    await expect(canvas.queryByText("Drawing the diagram…")).toBeNull()
  },
}
