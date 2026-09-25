import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantMarkdown } from "./assistant-markdown"
import { highlight } from "./code-highlight"
import { renderMermaid } from "./mermaid-render"
import { preloadBeforeStories } from "./story-preload"

// shiki with the two grammars these stories colour, and mermaid with the two
// diagram types they draw, loaded and compiled before the first story is
// timed. The samples differ from the stories' own blocks: each story still
// waits for its block to be coloured or drawn, it no longer pays for loading
// the libraries that do it.
preloadBeforeStories(() =>
  Promise.all([
    highlight("sql", "SELECT 1"),
    highlight("json", "{}"),
    renderMermaid("flowchart LR\n  a --> b", true),
    renderMermaid("sequenceDiagram\n  a->>b: c", false),
  ])
)

// With the libraries loaded, what is left is one block to tokenise or one
// diagram to lay out: milliseconds, yet seconds on a machine under heavy load,
// past `waitFor`'s default second. The bound stays under the story's 15 s
// budget, so a block that never renders fails on its own message rather than
// on the test's timeout.
const RENDERED_WITHIN = { timeout: 10_000 }

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
    await waitFor(
      () => expect(code).toHaveAttribute("data-highlighted", "true"),
      RENDERED_WITHIN
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
      RENDERED_WITHIN
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
        RENDERED_WITHIN
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
        RENDERED_WITHIN
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

const PROPOSED = "SELECT count(*) FROM clients WHERE active;"

async function openCodeMenu(canvasElement: HTMLElement) {
  await userEvent.pointer({
    keys: "[MouseRight]",
    target: within(canvasElement).getByLabelText("Proposed SQL"),
  })
  const page = within(document.body)
  await page.findByRole("menu")
  return page
}

/**
 * The context menu of a proposed statement holds `Copy code` and `Open in
 * console`, and nothing else: a proposal is never run by being made (I-07).
 */
export const CodeBlockMenuNeverRuns: Story = {
  args: { text: `\`\`\`sql\n${PROPOSED}\n\`\`\`` },
  play: async ({ canvasElement, args }) => {
    const page = await openCodeMenu(canvasElement)
    await expect(
      page.getAllByRole("menuitem").map((item) => item.textContent.trim())
    ).toEqual(["Copy code", "Open in console"])
    await userEvent.click(
      page.getByRole("menuitem", { name: "Open in console" })
    )
    await expect(args.onOpenSql).toHaveBeenCalledWith(PROPOSED)
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}

/**
 * Where the button cannot open the statement — its provenance cannot be
 * recorded —, the menu does not offer it either: only `Copy code` remains.
 */
export const CodeBlockMenuWithoutProvenance: Story = {
  args: {
    text: `\`\`\`sql\n${PROPOSED}\n\`\`\``,
    openSqlDisabledReason:
      "This answer cannot be opened in a console: Oxyn cannot record where it came from.",
  },
  play: async ({ canvasElement, args }) => {
    const page = await openCodeMenu(canvasElement)
    await expect(
      page.getAllByRole("menuitem").map((item) => item.textContent.trim())
    ).toEqual(["Copy code"])
    await userEvent.click(page.getByRole("menuitem", { name: "Copy code" }))
    await expect(args.onCopy).toHaveBeenCalledWith(PROPOSED)
    await waitFor(() => expect(page.queryByRole("menu")).toBeNull())
  },
}
