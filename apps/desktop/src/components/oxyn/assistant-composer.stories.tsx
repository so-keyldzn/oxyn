import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import {
  expect,
  fireEvent,
  fn,
  userEvent,
  waitFor,
  within,
} from "storybook/test"

import { AssistantComposer } from "./assistant-composer"
import { measureChips } from "./assistant-mention-measure"
import type {
  MentionChoice,
  MentionResults,
  MentionSource,
} from "./assistant-composer"
import type { Mention } from "@/lib/ipc/ai"

/** The prop's own signature: the backend answers now or later. */
type Submit = (
  question: string,
  mentions: Array<Mention>
) => boolean | Promise<boolean>

const ORDERS: MentionChoice = {
  key: "orders",
  kind: "table",
  label: "orders",
  detail: "public",
  mention: {
    kind: "relation",
    address: { catalog: null, namespace: "public", relation: "orders" },
    field: null,
  },
}

const CATALOG: ReadonlyArray<MentionChoice> = [
  ORDERS,
  {
    key: "order_lines.order_id",
    kind: "column",
    label: "order_lines.order_id",
    detail: "public",
    mention: {
      kind: "relation",
      address: { catalog: null, namespace: "public", relation: "order_lines" },
      field: "order_id",
    },
  },
  {
    key: "open_orders",
    kind: "view",
    label: "open_orders",
    detail: "reporting",
    mention: {
      kind: "relation",
      address: {
        catalog: null,
        namespace: "reporting",
        relation: "open_orders",
      },
      field: null,
    },
  },
  {
    key: "events",
    kind: "collection",
    label: "events",
    detail: "analytics",
    mention: {
      kind: "relation",
      address: { catalog: null, namespace: "analytics", relation: "events" },
      field: null,
    },
  },
  {
    key: "saved:monthly",
    kind: "savedQuery",
    label: "Monthly revenue",
    detail: null,
    mention: {
      kind: "savedQuery",
      document: "018f0000-0000-7000-8000-000000000001",
    },
  },
]

/**
 * The feature's search, played by a filter over a fixed list: what the user
 * types after `@` narrows it, as the local catalog search would.
 */
function useFixtureSearch(
  fixed: MentionResults | null,
  catalog: ReadonlyArray<MentionChoice>
): MentionSource {
  const [query, setQuery] = React.useState<string | null>(null)
  const results: MentionResults =
    fixed ??
    (query === null || query === ""
      ? { status: "idle" }
      : {
          status: "ready",
          choices: catalog.filter((choice) =>
            choice.label.toLowerCase().includes(query.toLowerCase())
          ),
        })
  return { results, onQuery: setQuery }
}

function WithMentions(
  props: React.ComponentProps<typeof AssistantComposer> & {
    fixed?: MentionResults
    catalog?: ReadonlyArray<MentionChoice>
  }
) {
  const { fixed, catalog, ...rest } = props
  const mentions = useFixtureSearch(fixed ?? null, catalog ?? CATALOG)
  return <AssistantComposer {...rest} mentions={mentions} />
}

const meta = {
  title: "Oxyn/Assistant/Composer",
  component: AssistantComposer,
  args: {
    running: false,
    onSubmit: fn<Submit>(() => true),
    onStop: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantComposer>

export default meta
type Story = StoryObj<typeof meta>

function fieldIn(canvasElement: HTMLElement) {
  return within(canvasElement).getByRole("textbox", {
    name: "Question for the assistant",
  })
}

/** The `@` list is portalled to the body, beside the caret. */
const list = () =>
  within(document.body).findByRole("listbox", { name: "Objects to mention" })

const listClosed = () =>
  waitFor(() =>
    expect(
      within(document.body).queryByRole("listbox", {
        name: "Objects to mention",
      })
    ).toBeNull()
  )

const chips = (canvasElement: HTMLElement) =>
  Array.from(
    canvasElement.querySelectorAll("[data-slot=assistant-mention]"),
    (chip) => chip.textContent
  )

export const Initial: Story = {
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "Which tables have no primary key?")
    // Shift+Enter is a new line, not a send.
    await userEvent.keyboard("{Shift>}{Enter}{/Shift}")
    await expect(args.onSubmit).not.toHaveBeenCalled()
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).toHaveBeenCalledWith(
      "Which tables have no primary key?",
      []
    )
    await waitFor(() => expect(field.textContent).toBe(""))
  },
}

export const Running: Story = {
  args: { running: true, initialValue: "And the indexes?" },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    // A message typed while an answer runs is queued, not refused — and it
    // approves nothing that is waiting for a review.
    await expect(canvas.getByText(/queues/)).toBeVisible()
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).toHaveBeenCalledWith("And the indexes?", [])
    await waitFor(() => expect(field.textContent).toBe(""))
    await userEvent.keyboard("{Escape}")
    await expect(args.onStop).toHaveBeenCalledOnce()
    await expect(
      canvas.getByRole("button", { name: "Stop the assistant" })
    ).toBeEnabled()
  },
}

export const StopRequested: Story = {
  args: { running: true, stopRequested: true },
}

/** Nothing can be asked: the draft stays readable, and nothing types into it. */
export const Unavailable: Story = {
  args: {
    initialValue: "Which tables have no primary key?",
    disabledReason:
      "This connection's privacy tier only admits a provider that resolved to this machine.",
  },
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await expect(field).toHaveAttribute("aria-readonly", "true")
    await userEvent.click(field)
    await userEvent.keyboard("more{Enter}")
    await expect(field.textContent).toBe("Which tables have no primary key?")
    await expect(args.onSubmit).not.toHaveBeenCalled()
  },
}

export const RefusedByBackend: Story = {
  args: {
    onSubmit: fn<Submit>(() => false),
    initialValue: "Drop the audit table",
  },
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).toHaveBeenCalled()
    // A refusal before anything started keeps the draft.
    await expect(field.textContent).toBe("Drop the audit table")
  },
}

export const DoubleSubmit: Story = {
  args: {
    onSubmit: fn<Submit>(
      () =>
        new Promise<boolean>((resolve) => {
          window.setTimeout(() => resolve(true), 300)
        })
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "Delete the archived invoices")
    // An impatient second Enter, then a click on Send, while the first is in
    // flight: a question asked twice is answered twice, and billed twice.
    await userEvent.keyboard("{Enter}{Enter}")
    await userEvent.click(canvas.getByRole("button", { name: "Send question" }))
    await waitFor(() => expect(field.textContent).toBe(""))
    await expect(args.onSubmit).toHaveBeenCalledTimes(1)
  },
}

/** An input method commits its text with Enter: nothing is sent. */
export const Composing: Story = {
  args: { initialValue: "日本語" },
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    fireEvent.keyDown(field, { key: "Enter", code: "Enter", isComposing: true })
    await expect(args.onSubmit).not.toHaveBeenCalled()
    await expect(field.textContent).toBe("日本語")
  },
}

export const NarrowWindow: Story = {
  args: { initialValue: "Which tables have no primary key?" },
  decorators: [
    (Story) => (
      <div className="w-[320px] p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // The hint and the send button share one row: neither may be pushed out.
    await expect(
      canvas.getByRole("button", { name: "Send question" })
    ).toBeVisible()
    await expect(canvasElement.scrollWidth).toBeLessThanOrEqual(
      canvasElement.clientWidth
    )
  },
}

/** `@` alone: the list opens and says what it searches. */
export const MentionListOpen: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "Count @")
    const shown = await list()
    await expect(within(shown).getByText(/Type to search tables/)).toBeVisible()
  },
}

/** What is typed after `@` filters the list, the caret staying in the field. */
export const MentionFiltering: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@ord")
    const shown = await list()
    await waitFor(() =>
      expect(
        within(shown)
          .getAllByRole("option")
          .filter((option) => option.getAttribute("aria-disabled") !== "true")
          .map((option) => option.textContent)
      ).toEqual([
        "orderspublic",
        "order_lines.order_idpublic",
        "open_ordersreporting",
      ])
    )
    // The field points at the highlighted option, and keeps the focus.
    await expect(field).toHaveFocus()
    await expect(field).toHaveAttribute("aria-activedescendant")
  },
}

/**
 * Enter in the list chooses the object — it never sends the question —, and
 * the chip carries the address, not the text.
 */
export const MentionChipInserted: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "Count rows in @orde")
    await list()
    await userEvent.keyboard("{Enter}")
    await expect(args.onSubmit).not.toHaveBeenCalled()
    await listClosed()
    await waitFor(() => expect(chips(canvasElement)).toEqual(["@orders"]))
    await userEvent.keyboard("please{Enter}")
    await expect(args.onSubmit).toHaveBeenCalledWith(
      "Count rows in @orders please",
      [ORDERS.mention]
    )
  },
}

/** Backspace takes the chip whole: no label left without its address. */
export const MentionRemovedByBackspace: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "Explain @orders")
    await list()
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(chips(canvasElement)).toEqual(["@orders"]))
    // The space after the chip, then the chip.
    await userEvent.keyboard("{Backspace}{Backspace}")
    await waitFor(() => expect(chips(canvasElement)).toEqual([]))
    await userEvent.keyboard("it{Enter}")
    await expect(args.onSubmit).toHaveBeenCalledWith("Explain it", [])
  },
}

/** No object matches: said, and Enter still sends nothing from the list. */
export const MentionNoResult: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@zzz")
    const shown = await list()
    await expect(within(shown).getByText("No matching object")).toBeVisible()
    await expect(
      within(shown)
        .getAllByRole("option")
        .map((option) => option.getAttribute("aria-disabled"))
    ).toEqual(["true"])
  },
}

export const MentionLoading: Story = {
  render: (args) => <WithMentions {...args} fixed={{ status: "loading" }} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@ord")
    const shown = await list()
    await expect(within(shown).getByText("Loading…")).toBeVisible()
    await expect(within(shown).queryByText("No matching object")).toBeNull()
  },
}

export const MentionFailed: Story = {
  render: (args) => (
    <WithMentions
      {...args}
      fixed={{
        status: "failed",
        message: "This connection has no catalog to search yet.",
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@ord")
    const shown = await list()
    await expect(
      within(shown).getByText("This connection has no catalog to search yet.")
    ).toBeVisible()
  },
}

/**
 * While an answer runs, Escape in the open list closes the list and stops
 * nothing; only the next Escape, in the field, stops the answer.
 */
export const EscapeClosesTheListFirst: Story = {
  args: { running: true },
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement, args }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@ord")
    await list()
    await userEvent.keyboard("{Escape}")
    await listClosed()
    await expect(args.onStop).not.toHaveBeenCalled()
    await userEvent.keyboard("{Escape}")
    await expect(args.onStop).toHaveBeenCalledOnce()
  },
}

/** Twenty tables: more than the list shows without scrolling. */
const MANY: ReadonlyArray<MentionChoice> = Array.from(
  { length: 20 },
  (_, index): MentionChoice => ({
    key: `orders_${index}`,
    kind: "table",
    label: `orders_${String(index).padStart(2, "0")}`,
    detail: "public",
    mention: {
      kind: "relation",
      address: {
        catalog: null,
        namespace: "public",
        relation: `orders_${index}`,
      },
      field: null,
    },
  })
)

const listBox = () =>
  document.querySelector<HTMLElement>("[data-slot=assistant-mention-list]")

/** The composer where the panel puts it: at the bottom of a tall column. */
function atTheBottom(height: string) {
  return (Story: () => React.JSX.Element) => (
    <div className="flex flex-col justify-end p-2" style={{ height }}>
      <Story />
    </div>
  )
}

/**
 * The list opens above the field — below it, the window ends — and stays
 * within the window and the field's width.
 */
export const MentionListOpensAbove: Story = {
  render: (args) => <WithMentions {...args} />,
  decorators: [atTheBottom("calc(100vh - 2rem)")],
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@ord")
    await list()
    const group = field.closest<HTMLElement>("[data-slot=input-group]")
    await waitFor(() => expect(listBox()?.dataset.side).toBe("top"))
    const box = listBox()?.getBoundingClientRect()
    const fieldBox = group?.getBoundingClientRect()
    await expect(box?.bottom).toBeLessThanOrEqual(fieldBox?.top ?? 0)
    await expect(box?.top).toBeGreaterThanOrEqual(0)
    await expect(box?.left).toBeGreaterThanOrEqual(fieldBox?.left ?? 0)
    await expect(box?.right).toBeLessThanOrEqual(fieldBox?.right ?? 0)
  },
}

/** A 360 px panel in a low window: the list shrinks and scrolls, inside it. */
export const MentionListInALowNarrowWindow: Story = {
  render: (args) => <WithMentions {...args} catalog={MANY} />,
  decorators: [
    (Story) => (
      <div className="w-[360px]">
        <Story />
      </div>
    ),
    atTheBottom("220px"),
  ],
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@orders")
    await list()
    const group = field.closest<HTMLElement>("[data-slot=input-group]")
    await waitFor(() => expect(listBox()).not.toBeNull())
    const shown = listBox()
    const box = shown?.getBoundingClientRect()
    const fieldBox = group?.getBoundingClientRect()
    await expect(box?.top).toBeGreaterThanOrEqual(0)
    await expect(box?.bottom).toBeLessThanOrEqual(window.innerHeight)
    await expect(box?.width).toBeLessThanOrEqual(fieldBox?.width ?? 0)
    // Bounded by the room there is, and scrolled inside.
    await expect(shown?.scrollHeight).toBeGreaterThan(shown?.clientHeight ?? 0)
  },
}

/** The active row stays in sight as the arrows move it through a long list. */
export const MentionActiveStaysVisible: Story = {
  render: (args) => <WithMentions {...args} catalog={MANY} />,
  decorators: [atTheBottom("calc(100vh - 2rem)")],
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@orders")
    // Once the list answers the whole query: on a loaded machine, an arrow
    // pressed earlier is undone when Lexical commits the last keystroke.
    const answering = await list()
    await waitFor(() =>
      expect(answering).toHaveAttribute("data-query", "orders")
    )
    // From the first row, up wraps to the last: far below the fold.
    await userEvent.keyboard("{ArrowUp}")
    const shown = listBox()
    const last = within(document.body).getByRole("option", {
      name: /orders_19/,
    })
    await waitFor(() => expect(last).toHaveAttribute("aria-selected", "true"))
    await waitFor(() => {
      const row = last.getBoundingClientRect()
      const frame = shown?.getBoundingClientRect()
      expect(row.top).toBeGreaterThanOrEqual(frame?.top ?? 0)
      expect(row.bottom).toBeLessThanOrEqual(frame?.bottom ?? 0)
    })
  },
}

/**
 * One active row, whoever moves it: the pointer takes it, the arrows move it
 * on from there, and the field points at the same one. Enter chooses it.
 */
export const MentionOneActiveRow: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "@or")
    const shown = await list()
    const rows = () =>
      within(shown)
        .getAllByRole("option")
        .filter((option) => option.getAttribute("aria-disabled") !== "true")
    const active = () =>
      rows().filter((row) => row.getAttribute("aria-selected") === "true")

    await userEvent.hover(rows()[2] ?? shown)
    await waitFor(() => expect(active()).toEqual([rows()[2]]))
    await expect(field).toHaveAttribute("aria-activedescendant", rows()[2]?.id)

    await userEvent.keyboard("{ArrowUp}")
    await waitFor(() => expect(active()).toEqual([rows()[1]]))
    await expect(shown.querySelectorAll("[data-active]").length).toBe(1)
    await expect(field).toHaveAttribute("aria-activedescendant", rows()[1]?.id)
    await userEvent.keyboard("{Enter}")
    await waitFor(() =>
      expect(chips(canvasElement)).toEqual(["@order_lines.order_id"])
    )
  },
}

/**
 * Typing `@` shows the list at once: the 100 ms of docs/PERFORMANCE.md, on the
 * component's side — the backend's local search is measured apart.
 */
export const MentionListOpensAtOnce: Story = {
  render: (args) => <WithMentions {...args} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.type(field, "Count ")
    const typed = performance.now()
    await userEvent.keyboard("@")
    await within(document.body).findByRole(
      "listbox",
      { name: "Objects to mention" },
      { interval: 5 }
    )
    await expect(performance.now() - typed).toBeLessThan(100)
  },
}

/** The caret's box where the selection sits now. */
function caretBox() {
  const selection = window.getSelection()
  if (selection === null || selection.rangeCount === 0) return null
  const rect = selection.getRangeAt(0).getClientRects()[0]
  return rect ?? null
}

/**
 * A chip reads as a word: same centre as the text beside it (1 px), no line
 * taller for holding one, the caret level on both sides — here on three
 * lines, one chip wrapped at a line's end.
 */
async function chipsSitInTheLine(canvasElement: HTMLElement) {
  const field = fieldIn(canvasElement)
  await userEvent.click(field)
  await userEvent.type(field, "Count ")
  const before = field.getBoundingClientRect().height
  await userEvent.type(field, "@orde")
  await list()
  await userEvent.keyboard("{Enter}")
  await waitFor(() => expect(chips(canvasElement)).toEqual(["@orders"]))
  // Inserting the chip moves nothing: the field keeps its height.
  await expect(field.getBoundingClientRect().height).toBe(before)
  // The caret after the chip stands level with it.
  const caret = caretBox()
  const chip = canvasElement.querySelector("[data-slot=assistant-mention]")
  if (caret === null || chip === null)
    throw new Error("a caret after the chip, and the chip")
  const box = chip.getBoundingClientRect()
  await expect(
    Math.abs(caret.top + caret.height / 2 - (box.top + box.height / 2))
  ).toBeLessThanOrEqual(1)
  // And before it, once the arrow has crossed it.
  await userEvent.keyboard("{ArrowLeft}")
  const left = caretBox()
  if (left === null) throw new Error("a caret before the chip")
  await expect(
    Math.abs(left.top + left.height / 2 - (box.top + box.height / 2))
  ).toBeLessThanOrEqual(1)
  await userEvent.keyboard("{ArrowRight}")

  await userEvent.type(
    field,
    "per month for the customers who ordered twice, then join @order_l"
  )
  await list()
  await userEvent.keyboard("{Enter}")
  await userEvent.type(field, " and @open")
  await list()
  await userEvent.keyboard("{Enter}")
  await userEvent.type(field, "please")
  await waitFor(() => expect(chips(canvasElement)).toHaveLength(3))

  const paragraph = field.querySelector("p")
  if (paragraph === null) throw new Error("Lexical renders a paragraph")
  const measured = await measureChips(paragraph)
  await expect(measured.chips).toBe(3)
  await expect(measured.alone).toBe(0)
  for (const offset of measured.offsets)
    await expect(offset).toBeLessThanOrEqual(1)
  // Several lines, and not one of them grown by a chip.
  await expect(measured.withChips).toBeGreaterThan(40)
  await expect(
    Math.abs(measured.withChips - measured.withoutChips)
  ).toBeLessThanOrEqual(0.5)
}

const narrow = (Story: () => React.ReactElement) => (
  <div className="w-[300px] p-4">
    <Story />
  </div>
)

export const MentionChipAlignment: Story = {
  render: (args) => <WithMentions {...args} />,
  decorators: [narrow],
  play: ({ canvasElement }) => chipsSitInTheLine(canvasElement),
}

export const MentionChipAlignmentLight: Story = {
  ...MentionChipAlignment,
  globals: { theme: "light" },
}

/**
 * The feature's source with its caches: `warm` answers from the first `@`, as
 * when the sidebar has read the tree; otherwise the first answer takes
 * `delay`, as a slow backend would, and every later one is at once.
 */
function useCachedSearch(warm: boolean, delay: number): MentionSource {
  const [query, setQuery] = React.useState<string | null>(null)
  const [answered, setAnswered] = React.useState(warm)
  React.useEffect(() => {
    if (answered || query === null) return
    const timer = window.setTimeout(() => setAnswered(true), delay)
    return () => window.clearTimeout(timer)
  }, [answered, query, delay])
  const results: MentionResults =
    query === null
      ? { status: "idle" }
      : !answered
        ? { status: "loading" }
        : {
            status: "ready",
            choices: CATALOG.filter((choice) =>
              choice.label.toLowerCase().includes(query.toLowerCase())
            ),
          }
  return { results, onQuery: setQuery }
}

function WithCache(
  props: React.ComponentProps<typeof AssistantComposer> & { warm: boolean }
) {
  const { warm, ...rest } = props
  const mentions = useCachedSearch(warm, 400)
  return <AssistantComposer {...rest} mentions={mentions} />
}

/** How long the first `@` takes to show its first object, in ms. */
async function firstObjectsAfterAt(canvasElement: HTMLElement) {
  const field = fieldIn(canvasElement)
  await userEvent.click(field)
  const typed = performance.now()
  await userEvent.keyboard("@")
  const shown = await list()
  await waitFor(
    () =>
      expect(within(shown).getAllByRole("option")[0]).toHaveTextContent(
        "orders"
      ),
    { interval: 5, timeout: 2000 }
  )
  return { shown, elapsed: performance.now() - typed }
}

/**
 * First `@`, the catalog already in the cache: the objects are there at the
 * keystroke — under the 100 ms of docs/PERFORMANCE.md — with no invitation to
 * type first.
 */
export const FirstMentionWarmCache: Story = {
  render: (args) => <WithCache {...args} warm />,
  play: async ({ canvasElement }) => {
    const { shown, elapsed } = await firstObjectsAfterAt(canvasElement)
    await expect(elapsed).toBeLessThan(100)
    await expect(within(shown).queryByText("Loading…")).toBeNull()
  },
}

/**
 * First `@` on a cold cache: a loading row at once — never "No matching
 * object" — then the objects when the backend answers.
 */
export const FirstMentionColdCache: Story = {
  render: (args) => <WithCache {...args} warm={false} />,
  play: async ({ canvasElement }) => {
    const field = fieldIn(canvasElement)
    await userEvent.click(field)
    await userEvent.keyboard("@")
    const shown = await list()
    await expect(within(shown).getByText("Loading…")).toBeVisible()
    await expect(within(shown).queryByText("No matching object")).toBeNull()
    await waitFor(
      () =>
        expect(within(shown).getAllByRole("option")[0]).toHaveTextContent(
          "orders"
        ),
      { timeout: 2000 }
    )
    await expect(within(shown).queryByText("Loading…")).toBeNull()
  },
}
