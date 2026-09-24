import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor, within } from "storybook/test"

import { ResultChartView } from "./assistant-result-chart"
import { chartData, chartPlan } from "./result-chart-model"
import type { ChartData } from "./result-chart-model"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

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

const totals = read(
  [column("orders", "Int64"), column("revenue", "Decimal128(12, 2)")],
  [[`4${GROUP}823`, `1${GROUP}204${GROUP}318.40`]]
)

const meta = {
  title: "Oxyn/Assistant/ResultChart",
  component: ResultChartView,
  decorators: [
    (Story) => (
      <div className="w-[420px] overflow-hidden rounded-lg border bg-card">
        <Story />
      </div>
    ),
  ],
  args: { data: byStatus },
} satisfies Meta<typeof ResultChartView>

export default meta
type Story = StoryObj<typeof meta>

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
    // A scatter needs two rows: listed, refused, and saying why.
    const scatter = canvas.getByRole("button", { name: "Scatter" })
    await expect(scatter).toBeDisabled()
    await expect(scatter).toHaveAccessibleDescription(
      "Fewer than two rows hold both numbers."
    )
  },
}

/** One series over dates: Oxyn fills an area. */
export const Area: Story = {
  args: { data: byDay(1) },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-area-area")
    await expect(
      canvas.getByRole("button", { name: "Automatic shape" })
    ).toHaveAttribute("aria-pressed", "true")
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
    // Categories are not a trend: no line, and the reason is given.
    await expect(
      canvas.getByRole("button", { name: "Line" })
    ).toHaveAccessibleDescription(/would draw a trend they do not have/)
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

/** A negative margin has no share of a whole: the pie stays listed, refused. */
export const PieRefused: Story = {
  args: { data: margins },
  play: async ({ canvas }) => {
    const pie = canvas.getByRole("button", { name: "Pie" })
    await expect(pie).toBeDisabled()
    await expect(pie).toHaveAccessibleDescription(
      "A value is negative: it cannot be part of a whole."
    )
    await expect(
      canvas.getByText("Drawn as Vertical bars, chosen by Oxyn.")
    ).toBeInTheDocument()
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

/** Two numbers and no axis: one against the other. */
export const Scatter: Story = {
  args: { data: priceAgainstSales },
  play: async ({ canvas, canvasElement }) => {
    await marks(canvasElement, ".recharts-scatter-symbol")
    await expect(canvas.getByText("units against price")).toBeVisible()
  },
}

/** The user's choice: a family, then a variant; « Auto » gives Oxyn's back. */
export const Choosing: Story = {
  args: { data: byStatus },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Bar" }))
    await expect(
      canvas.getByText("Drawn as Vertical bars.")
    ).toBeInTheDocument()
    await userEvent.click(
      canvas.getByRole("combobox", { name: "Chart variant: Vertical bars" })
    )
    const list = await within(document.body).findByRole("listbox")
    // Refused variants are listed, disabled, with their reason under the name.
    await expect(
      within(list).getByRole("option", { name: /Grouped bars/ })
    ).toHaveAttribute("aria-disabled", "true")
    await expect(
      within(list).getByText("One numeric column: nothing to group.")
    ).toBeInTheDocument()
    await userEvent.click(
      within(list).getByRole("option", { name: /Horizontal bars/ })
    )
    await expect(
      canvas.getByText("Drawn as Horizontal bars.")
    ).toBeInTheDocument()
    // Base UI leaves its list box unnamed: it must be gone before axe looks.
    await waitFor(() =>
      expect(within(document.body).queryByRole("listbox")).toBeNull()
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Automatic shape" })
    )
    await expect(
      canvas.getByText("Drawn as Donut, chosen by Oxyn.")
    ).toBeInTheDocument()
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
export const ScatterLight: Story = { ...Scatter, ...light }
