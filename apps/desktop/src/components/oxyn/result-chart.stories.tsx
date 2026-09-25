import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor, within } from "storybook/test"

import { ResultChartView } from "./assistant-result-chart"
import { chartData, chartPlan } from "./result-chart-model"
import type { ChartData } from "./result-chart-model"
import { ASIDE_WIDTH } from "./workspace-layout"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

type Canvas = ReturnType<typeof within>

/** What the chart reads from these columns and rows, as the component would. */
function read(
  columns: Array<ResultColumn>,
  rows: Array<Array<Cell>>
): ChartData {
  const plan = chartPlan(columns, rows.length)
  const data = plan ? chartData(plan, columns, rows) : null
  if (!data) throw new Error("these rows draw nothing")
  return data
}

const column = (name: string, dataType: string): ResultColumn => ({
  name,
  dataType,
  nullable: true,
})

// Rust groups digits by U+00A0, spelled by its code: written out, it is
// invisible in the source.
const GROUP = String.fromCharCode(0xa0)

const byDay = (series: number) =>
  read(
    [
      column("day", "Date32"),
      column("signups", "Int32"),
      column("trials", "Int32"),
    ].slice(0, series + 1),
    Array.from({ length: 14 }, (_, index): Array<Cell> =>
      [
        `2026-09-${String(index + 1).padStart(2, "0")}`,
        String(40 + ((index * 17) % 23)),
        String(12 + ((index * 7) % 11)),
      ].slice(0, series + 1)
    )
  )

const byStatus = read(
  [column("status", "Utf8"), column("orders", "Int64")],
  [
    ["shipped", `1${GROUP}204`],
    ["pending", "312"],
    ["cancelled", "58"],
    ["returned", "41"],
  ]
)

const byRegion = read(
  [
    column("region", "Utf8"),
    column("orders", "Int64"),
    column("returns", "Int64"),
  ],
  [
    ["North", "120", "14"],
    ["South", "98", "22"],
    ["East", "143", "9"],
    ["West", "87", "17"],
    ["Centre", "110", "12"],
  ]
)

const byCountry = read(
  [column("country", "Utf8"), column("customers", "Int64")],
  [
    "France",
    "Germany",
    "United Kingdom of Great Britain and Northern Ireland",
    "Spain",
    "Italy",
    "Portugal",
    "Belgium",
    "Netherlands",
    "Switzerland",
    "Austria",
    "Poland",
    "Sweden",
    "Norway",
    "Denmark",
    "Finland",
  ].map((name, index) => [name, String(900 - index * 53)])
)

const margins = read(
  [column("status", "Utf8"), column("margin", "Decimal128(8, 2)")],
  [
    ["shipped", "12.50"],
    ["refunded", "-4.20"],
    ["pending", "3.10"],
  ]
)

const priceAgainstSales = read(
  [column("price", "Decimal128(8, 2)"), column("units", "Int64")],
  Array.from({ length: 24 }, (_, index): Array<Cell> => [
    (4 + index * 1.5).toFixed(2),
    String(Math.round(400 / (1 + index * 0.4)) + ((index * 13) % 17)),
  ])
)

/**
 * The query reported on 2026-09-25: SQLite's `AVG` is a REAL, a `Float64`
 * column, and Rust writes each value as plain decimal text. Four averages in
 * four units — not parts of one whole.
 */
const averages = read(
  [column("indicateur", "Utf8"), column("moyenne", "Float64")],
  [
    ["Montant moyen d'une commande", "187.4213"],
    ["Points de fidélité moyens par client", `1${GROUP}245.5`],
    ["Prix moyen d'un produit", "42.9"],
    ["Note moyenne des avis", "3.86"],
  ]
)

const totals = read(
  [column("orders", "Int64"), column("revenue", "Decimal128(12, 2)")],
  [[`4${GROUP}823`, `1${GROUP}204${GROUP}318.40`]]
)

const meta = {
  title: "Oxyn/Assistant/ResultChart",
  component: ResultChartView,
  decorators: [
    (Story, { parameters }) => (
      <div
        className="overflow-hidden rounded-lg border bg-card"
        style={{ width: (parameters.width as number | undefined) ?? 420 }}
      >
        <Story />
      </div>
    ),
  ],
  args: { data: byStatus },
} satisfies Meta<typeof ResultChartView>

export default meta
type Story = StoryObj<typeof meta>

/** The shape menu, opened: its list box, portalled out of the canvas. */
async function openShapes(canvas: Canvas) {
  await userEvent.click(canvas.getByRole("combobox", { name: /^Chart shape/ }))
  return within(document.body).findByRole("listbox")
}

/** Base UI leaves its list box unnamed: it must be gone before axe looks. */
async function closeShapes() {
  await userEvent.keyboard("{Escape}")
  await waitFor(() =>
    expect(within(document.body).queryByRole("listbox")).toBeNull()
  )
}

/** The drawing, once Recharts has measured its box: marks, not an empty frame. */
async function marks(root: HTMLElement, selector: string) {
  await waitFor(() =>
    expect(root.querySelectorAll(selector).length).toBeGreaterThan(0)
  )
}

/** One row is a record: its numbers, as Rust wrote them, not a chart of one bar. */
export const KeyFigure: Story = {
  args: { data: totals },
  play: async ({ canvas }) => {
    // Rust's text, grouping space included: the matcher would fold it.
    await expect(canvas.getByText(/318\.40$/).textContent).toBe(
      `1${GROUP}204${GROUP}318.40`
    )
    await expect(
      canvas.getByText("Drawn as Key figure, chosen by Oxyn.")
    ).toBeInTheDocument()
    // A scatter needs two rows: not offered.
    const list = await openShapes(canvas)
    await expect(
      within(list).queryByRole("option", { name: "Scatter" })
    ).toBeNull()
    await closeShapes()
  },
}

/** One series over dates: Oxyn fills an area. */
export const Area: Story = {
  args: { data: byDay(1) },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-area-area")
    // One control, naming only what is drawn.
    await expect(
      canvas.getByRole("combobox", { name: "Chart shape: Auto · Area" })
    ).toBeVisible()
    await expect(
      canvas.getByText("Drawn as Area, chosen by Oxyn.")
    ).toBeInTheDocument()
    await expect(
      canvas.getByText("signups by day, in the order the query returned")
    ).toBeVisible()
  },
}

export const AreaStackedExpanded: Story = {
  args: { data: byDay(2), initialChoice: "area-expanded" },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/each point as a share of its row's total/)
    ).toBeVisible()
  },
}

export const AreaGradient: Story = {
  args: { data: byDay(2), initialChoice: "area-gradient" },
  play: async ({ canvasElement }) => {
    await marks(canvasElement, "linearGradient stop")
  },
}

/** Several series over dates: lines, since areas would hide each other. */
export const Line: Story = {
  args: { data: byDay(2) },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-line-curve")
    await expect(
      canvas.getByText("Drawn as Curved line, chosen by Oxyn.")
    ).toBeInTheDocument()
  },
}

export const LineWithPoints: Story = {
  args: { data: byDay(1), initialChoice: "line-dots" },
  play: async ({ canvasElement }) => {
    await marks(canvasElement, ".recharts-line-dot")
  },
}

/** Several series on a few categories: grouped bars. */
export const Bar: Story = {
  args: { data: byRegion },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-bar-rectangle")
    await expect(
      canvas.getByText("Drawn as Grouped bars, chosen by Oxyn.")
    ).toBeInTheDocument()
    // Categories are not a trend: no line is offered, nor its family.
    const list = await openShapes(canvas)
    await expect(
      within(list).queryByRole("option", { name: "Straight line" })
    ).toBeNull()
    await expect(within(list).queryByText("Line")).toBeNull()
    await expect(
      within(list).getByRole("option", { name: "Grouped bars" })
    ).toBeInTheDocument()
    await closeShapes()
  },
}

/** Many categories and a long name: bars lie down, every label on its line. */
export const BarHorizontal: Story = {
  args: { data: byCountry },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-bar-rectangle")
    await expect(
      canvas.getByText("Drawn as Horizontal bars, chosen by Oxyn.")
    ).toBeInTheDocument()
    await expect(
      canvas.getByRole("group", { name: "Chart, scrollable" })
    ).toHaveAttribute("tabindex", "0")
  },
}

export const BarStacked: Story = {
  args: { data: byRegion, initialChoice: "bar-stacked" },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/stacked: the top edge is their sum/)
    ).toBeVisible()
  },
}

export const BarWithValues: Story = {
  args: { data: byStatus, initialChoice: "bar-label" },
  play: async ({ canvasElement }) => {
    await marks(canvasElement, ".recharts-label-list")
  },
}

/** One positive series of four parts: a donut, largest share first. */
export const Pie: Story = {
  args: { data: byStatus },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-pie-sector")
    await expect(
      canvas.getByText("Drawn as Donut, chosen by Oxyn.")
    ).toBeInTheDocument()
    await expect(
      canvas.getByText("orders by status, largest share first")
    ).toBeVisible()
    await expect(canvas.getByText("returned")).toBeVisible()
  },
}

export const DonutWithTotal: Story = {
  args: { data: byStatus, initialChoice: "donut-total" },
  play: async ({ canvas }) => {
    // 1 204 + 312 + 58 + 41, the only sum the chart writes.
    await expect(await canvas.findByText("1,615")).toBeInTheDocument()
  },
}

/** A negative margin has no share of a whole: no pie is offered. */
export const PieRefused: Story = {
  args: { data: margins },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Drawn as Vertical bars, chosen by Oxyn.")
    ).toBeInTheDocument()
    const list = await openShapes(canvas)
    await expect(within(list).queryByRole("option", { name: "Pie" })).toBeNull()
    await closeShapes()
  },
}

/** Never Oxyn's pick; one click away when the rows allow it. */
export const Radar: Story = {
  args: { data: byRegion, initialChoice: "radar" },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-radar-polygon")
    await expect(canvas.getByText("Drawn as Radar.")).toBeInTheDocument()
  },
}

export const Radial: Story = {
  args: { data: byStatus, initialChoice: "radial" },
  play: async ({ canvasElement }) => {
    await marks(canvasElement, ".recharts-radial-bar-sector")
  },
}

/** The assistant's panel as it opens: `ASIDE_WIDTH.initial`. */
const narrow = { parameters: { width: ASIDE_WIDTH.initial } }

/**
 * The drawing and each legend entry, as laid out: no entry may cross the
 * drawing's box, and the drawing must have room for its marks.
 */
async function legendApart(root: HTMLElement) {
  const figure = root.querySelector<HTMLElement>("[data-slot=chart]")
  const entries = root.querySelectorAll<HTMLElement>("li")
  await waitFor(() => {
    const box = figure?.getBoundingClientRect()
    expect(box?.height ?? 0).toBeGreaterThan(120)
    expect(entries.length).toBeGreaterThan(0)
    for (const entry of entries) {
      const at = entry.getBoundingClientRect()
      const apart =
        box !== undefined &&
        (at.top >= box.bottom ||
          at.bottom <= box.top ||
          at.left >= box.right ||
          at.right <= box.left)
      expect(apart).toBe(true)
    }
  })
}

/**
 * Four averages in four units: not shares of a whole, so not a donut —
 * bars, which lie down for labels this long.
 */
export const Averages: Story = {
  args: { data: averages },
  ...narrow,
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-bar-rectangle")
    await expect(
      canvas.getByText("Drawn as Horizontal bars, chosen by Oxyn.")
    ).toBeInTheDocument()
  },
}

/** The user asks for a donut anyway: a ring, and a legend under it. */
export const DonutLongLabelsNarrow: Story = {
  args: { data: averages, initialChoice: "donut" },
  ...narrow,
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-pie-sector")
    await legendApart(canvasElement)
    await expect(
      canvas.getByText("Points de fidélité moyens par client")
    ).toBeVisible()
  },
}

/** A long total in a narrow panel: written smaller, inside the hole. */
export const DonutTotalNarrow: Story = {
  args: { data: averages, initialChoice: "donut-total" },
  ...narrow,
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-pie-sector")
    await legendApart(canvasElement)
    const figure = await canvas.findByText("1,479.681")
    const box = canvasElement
      .querySelector("[data-slot=chart]")
      ?.getBoundingClientRect()
    // The hole is 48 % of a radius of 80 % of half the box.
    const hole = (box?.width ?? 0) * 0.8 * 0.48
    const width = figure.getBoundingClientRect().width
    await expect(width).toBeGreaterThan(hole / 2)
    await expect(width).toBeLessThan(hole)
  },
}

export const RadialLongLabelsNarrow: Story = {
  args: { data: averages, initialChoice: "radial" },
  ...narrow,
  play: async ({ canvasElement }) => {
    await marks(canvasElement, ".recharts-radial-bar-sector")
    await legendApart(canvasElement)
  },
}

/** Room enough: the legend stands beside the drawing. */
export const RadialLongLabelsWide: Story = {
  args: { data: averages, initialChoice: "radial" },
  parameters: { width: 640 },
  play: async ({ canvasElement }) => {
    await marks(canvasElement, ".recharts-radial-bar-sector")
    await legendApart(canvasElement)
    const box = canvasElement
      .querySelector("[data-slot=chart]")
      ?.getBoundingClientRect()
    const entry = canvasElement.querySelector("li")?.getBoundingClientRect()
    await expect((entry?.left ?? 0) >= (box?.right ?? 0)).toBe(true)
  },
}

/** Two numbers and no axis: one against the other. */
export const Scatter: Story = {
  args: { data: priceAgainstSales },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-scatter-symbol")
    await expect(canvas.getByText("units against price")).toBeVisible()
  },
}

/** The user's choice, in one menu; « Auto » gives Oxyn's back. */
export const Choosing: Story = {
  args: { data: byStatus },
  play: async ({ canvas }) => {
    let list = await openShapes(canvas)
    // A shape these rows cannot draw is not offered.
    await expect(
      within(list).queryByRole("option", { name: "Grouped bars" })
    ).toBeNull()
    await userEvent.click(
      within(list).getByRole("option", { name: "Horizontal bars" })
    )
    await expect(
      canvas.getByText("Drawn as Horizontal bars.")
    ).toBeInTheDocument()
    await waitFor(() =>
      expect(within(document.body).queryByRole("listbox")).toBeNull()
    )
    list = await openShapes(canvas)
    await userEvent.click(within(list).getByRole("option", { name: "Auto" }))
    await expect(
      canvas.getByText("Drawn as Donut, chosen by Oxyn.")
    ).toBeInTheDocument()
    await waitFor(() =>
      expect(within(document.body).queryByRole("listbox")).toBeNull()
    )
  },
}

const light = { globals: { theme: "light" } }

export const KeyFigureLight: Story = { ...KeyFigure, ...light }
export const AreaLight: Story = { ...Area, ...light }
export const LineLight: Story = { ...Line, ...light }
export const BarLight: Story = { ...Bar, ...light }
export const BarHorizontalLight: Story = { ...BarHorizontal, ...light }
export const PieLight: Story = { ...Pie, ...light }
export const DonutWithTotalLight: Story = { ...DonutWithTotal, ...light }
export const RadarLight: Story = { ...Radar, ...light }
export const RadialLight: Story = { ...Radial, ...light }
export const DonutLongLabelsNarrowLight: Story = {
  ...DonutLongLabelsNarrow,
  ...light,
}
export const DonutTotalNarrowLight: Story = { ...DonutTotalNarrow, ...light }
export const RadialLongLabelsNarrowLight: Story = {
  ...RadialLongLabelsNarrow,
  ...light,
}
export const ScatterLight: Story = { ...Scatter, ...light }
