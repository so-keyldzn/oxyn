// Which shapes can draw a result honestly, which one Oxyn picks, and why.
//
// Every shape of shadcn's chart gallery is judged; the ones these rows would
// misdraw are not offered, and their reason says why. A reason is a fact
// about the rows ("a value is negative"), not a taste: taste is `autoShape`'s.

import type { ChartData, ChartRow } from "@/components/oxyn/result-chart-model"

export const CHART_FAMILIES = [
  "kpi",
  "area",
  "bar",
  "line",
  "pie",
  "radar",
  "radial",
  "scatter",
] as const
export type ChartFamily = (typeof CHART_FAMILIES)[number]

/** Every shape, grouped by family, in the order the picker lists them. */
export const CHART_SHAPES = [
  "kpi",
  "area",
  "area-stacked",
  "area-expanded",
  "area-step",
  "area-gradient",
  "bar-vertical",
  "bar-horizontal",
  "bar-grouped",
  "bar-stacked",
  "bar-label",
  "line-linear",
  "line-curved",
  "line-step",
  "line-dots",
  "pie",
  "donut",
  "donut-total",
  "radar",
  "radial",
  "scatter",
] as const
export type ChartShape = (typeof CHART_SHAPES)[number]

export const FAMILY_LABEL: Record<ChartFamily, string> = {
  kpi: "Key figure",
  area: "Area",
  bar: "Bar",
  line: "Line",
  pie: "Pie",
  radar: "Radar",
  radial: "Radial",
  scatter: "Scatter",
}

export const SHAPE_FAMILY: Record<ChartShape, ChartFamily> = {
  kpi: "kpi",
  area: "area",
  "area-stacked": "area",
  "area-expanded": "area",
  "area-step": "area",
  "area-gradient": "area",
  "bar-vertical": "bar",
  "bar-horizontal": "bar",
  "bar-grouped": "bar",
  "bar-stacked": "bar",
  "bar-label": "bar",
  "line-linear": "line",
  "line-curved": "line",
  "line-step": "line",
  "line-dots": "line",
  pie: "pie",
  donut: "pie",
  "donut-total": "pie",
  radar: "radar",
  radial: "radial",
  scatter: "scatter",
}

export const SHAPE_LABEL: Record<ChartShape, string> = {
  kpi: "Key figure",
  area: "Area",
  "area-stacked": "Stacked area",
  "area-expanded": "Stacked area, 100 %",
  "area-step": "Step area",
  "area-gradient": "Gradient area",
  "bar-vertical": "Vertical bars",
  "bar-horizontal": "Horizontal bars",
  "bar-grouped": "Grouped bars",
  "bar-stacked": "Stacked bars",
  "bar-label": "Bars with values",
  "line-linear": "Straight line",
  "line-curved": "Curved line",
  "line-step": "Step line",
  "line-dots": "Line with points",
  pie: "Pie",
  donut: "Donut",
  "donut-total": "Donut with total",
  radar: "Radar",
  radial: "Radial bars",
  scatter: "Scatter",
}

export type ShapeVerdict =
  { possible: true } | { possible: false; reason: string }

/**
 * `--chart-1` to `--chart-5`: a sixth slice would repeat a colour, and two
 * parts of the whole would read as one.
 */
export const PALETTE_SIZE = 5

/** Above this many bars, the values written on them overlap. */
const LABELLED_BARS = 12
/**
 * Above this many categories, or labels this long, vertical bars skip their
 * labels: a panel about 400 px wide gives each of a dozen bars some 30 px,
 * four or five characters. Horizontal bars give each label its own line.
 */
const VERTICAL_CATEGORIES = 12
const VERTICAL_LABEL = 14
/** A radar under three spokes is a line; over eight, its spokes blur. */
const RADAR_MIN = 3
const RADAR_MAX = 8

interface Facts {
  rows: number
  series: number
  axis: "temporal" | "category" | null
  hasNull: boolean
  hasNegative: boolean
  hasZero: boolean
  zeroTotal: boolean
  distinctLabels: boolean
  longestLabel: number
  scatterPoints: number
}

function values(data: ChartData, row: ChartRow): Array<number | null> {
  return data.series.map(({ key }) => {
    const value = row[key]
    return typeof value === "number" ? value : null
  })
}

function factsOf(data: ChartData): Facts {
  const all = data.rows.flatMap((row) => values(data, row))
  const labels = data.rows.map((row) => String(row.axis ?? ""))
  return {
    rows: data.rows.length,
    series: data.series.length,
    axis: data.axis?.kind ?? null,
    hasNull: all.some((value) => value === null),
    hasNegative: all.some((value) => value !== null && value < 0),
    hasZero: all.some((value) => value === 0),
    zeroTotal: data.rows.some((row) =>
      values(data, row).every((value) => (value ?? 0) === 0)
    ),
    distinctLabels: new Set(labels).size === labels.length,
    longestLabel: Math.max(0, ...labels.map((label) => label.length)),
    scatterPoints: scatterRows(data).length,
  }
}

type Check = [failed: boolean, reason: string]

/** The first check that fails, or `possible`. */
function verdict(checks: Array<Check>): ShapeVerdict {
  const failed = checks.find(([fails]) => fails)
  return failed ? { possible: false, reason: failed[1] } : { possible: true }
}

function checksFor(shape: ChartShape, facts: Facts): Array<Check> {
  const noAxis: Check = [
    facts.axis === null,
    "No date or category column to lay the values along.",
  ]
  const trend: Array<Check> = [
    noAxis,
    [
      facts.axis === "category",
      "The axis holds categories: a line between them would draw a trend they do not have.",
    ],
    [facts.rows < 2, "A line needs at least two points; there is one row."],
  ]
  // A stack reads as a sum: a negative crosses the bands, a NULL drops the
  // band to zero where the value is unknown.
  const stack: Array<Check> = [
    [facts.series < 2, "Stacking needs two numeric columns; there is one."],
    [facts.hasNegative, "A value is negative: stacked bands would cross."],
    [facts.hasNull, "A value is NULL: the stack would treat it as zero."],
  ]
  const oneSeries: Check = [
    facts.series !== 1,
    `It draws one numeric column; these rows have ${facts.series}.`,
  ]
  const categories: Check = [
    facts.axis !== "category",
    facts.axis === "temporal"
      ? "The axis holds dates: a sequence, not parts to compare side by side."
      : "No category column to name the parts.",
  ]
  const slices: Array<Check> = [
    categories,
    oneSeries,
    [facts.rows < 2, "One category is the whole: nothing to compare."],
    [
      facts.rows > PALETTE_SIZE,
      `${facts.rows} categories: the palette has ${PALETTE_SIZE} colours, and parts would share one.`,
    ],
    [
      facts.hasNull,
      "A value is NULL: it has no share, and the pie would hide it.",
    ],
    [facts.hasNegative, "A value is negative: it cannot be part of a whole."],
  ]
  switch (shape) {
    case "kpi":
      return [
        [
          facts.rows !== 1,
          `A key figure reads one row; these are ${facts.rows}.`,
        ],
      ]
    case "area":
    case "area-step":
    case "area-gradient":
    case "line-linear":
    case "line-curved":
    case "line-step":
    case "line-dots":
      return trend
    case "area-stacked":
      return [...trend, ...stack]
    case "area-expanded":
      return [
        ...trend,
        ...stack,
        [facts.zeroTotal, "A row sums to zero: it has no share to split."],
      ]
    case "bar-vertical":
      return [
        noAxis,
        [
          facts.series > 1,
          "Several numeric columns side by side: that is grouped bars.",
        ],
      ]
    case "bar-horizontal":
      return [noAxis]
    case "bar-grouped":
      return [
        noAxis,
        [facts.series < 2, "One numeric column: nothing to group."],
      ]
    case "bar-stacked":
      return [noAxis, ...stack]
    case "bar-label":
      return [
        noAxis,
        [
          facts.rows * facts.series > LABELLED_BARS,
          `${facts.rows * facts.series} bars: the values written on them would overlap.`,
        ],
      ]
    case "pie":
    case "donut":
    case "donut-total":
      return [
        ...slices,
        [
          facts.hasZero,
          "A value is zero: it draws no slice, and its category would vanish.",
        ],
        [
          !facts.distinctLabels,
          "A category appears twice: the slices would not be parts of one whole.",
        ],
      ]
    case "radial":
      return [
        categories,
        oneSeries,
        [facts.rows < 2, "One category: nothing to compare."],
        [
          facts.rows > PALETTE_SIZE,
          `${facts.rows} categories: the palette has ${PALETTE_SIZE} colours, and bars would share one.`,
        ],
        [facts.hasNull, "A value is NULL: its bar would read as zero."],
        [
          facts.hasNegative,
          "A value is negative: a radial bar cannot go below its start.",
        ],
      ]
    case "radar":
      return [
        categories,
        [
          facts.rows < RADAR_MIN || facts.rows > RADAR_MAX,
          `A radar needs ${RADAR_MIN} to ${RADAR_MAX} categories; these are ${facts.rows}.`,
        ],
        [facts.hasNull, "A value is NULL: the outline would break."],
        [
          facts.hasNegative,
          "A value is negative: a radar measures from its centre.",
        ],
      ]
    case "scatter":
      return [
        [
          facts.series < 2,
          "A scatter needs two numeric columns; there is one.",
        ],
        [facts.scatterPoints < 2, "Fewer than two rows hold both numbers."],
      ]
  }
}

/** For each shape, whether these rows can be drawn by it, and if not, why. */
export function shapeVerdicts(
  data: ChartData
): Record<ChartShape, ShapeVerdict> {
  const facts = factsOf(data)
  return Object.fromEntries(
    CHART_SHAPES.map((shape) => [shape, verdict(checksFor(shape, facts))])
  ) as Record<ChartShape, ShapeVerdict>
}

/**
 * The shape Oxyn draws when the user has not chosen one, `null` when none is
 * possible. The reasons, in the order they are tried:
 *
 * 1. one row is a record, not a series: its numbers are read as key figures;
 * 2. no axis, two numbers: a scatter, the only way to set them against each other;
 * 3. a date axis is a trend: one series fills an area, which reads as volume
 *    over time; several are lines, since overlapping areas hide each other;
 * 4. on categories, one positive series of 2 to 5 distinct parts is a donut —
 *    the share is the point, and the hole leaves room to read it;
 * 5. many categories or long labels turn bars horizontal, so every label is
 *    written; otherwise bars stand vertical, grouped when there are several;
 * 6. whatever remains possible, in the picker's order.
 *
 * Radar is never picked: it lays every column on one radial scale, and the
 * columns of a query rarely share a unit — grouped bars compare the same
 * values along a straight, common baseline. It stays one click away.
 */
export function autoShape(
  data: ChartData,
  verdicts: Record<ChartShape, ShapeVerdict>
): ChartShape | null {
  const can = (shape: ChartShape) => verdicts[shape].possible
  const facts = factsOf(data)
  const preferred: Array<ChartShape> = ["kpi"]
  if (facts.axis === null) preferred.push("scatter")
  else if (facts.axis === "temporal")
    preferred.push(facts.series === 1 ? "area" : "line-curved")
  else {
    preferred.push("donut")
    if (facts.rows > VERTICAL_CATEGORIES || facts.longestLabel > VERTICAL_LABEL)
      preferred.push("bar-horizontal")
    preferred.push(facts.series === 1 ? "bar-vertical" : "bar-grouped")
  }
  return preferred.find(can) ?? CHART_SHAPES.find((shape) => can(shape)) ?? null
}

/**
 * The shape to draw: the user's choice while these rows allow it, Oxyn's
 * otherwise — a choice made on other rows is not a promise about these.
 */
export function shapeToDraw(
  choice: ChartShape | "auto",
  auto: ChartShape | null,
  verdicts: Record<ChartShape, ShapeVerdict>
): ChartShape | null {
  return choice !== "auto" && verdicts[choice].possible ? choice : auto
}

/** The rows that hold both numbers of a scatter, in the query's order. */
export function scatterRows(data: ChartData): Array<ChartRow> {
  return data.rows.filter(
    (row) => typeof row.s0 === "number" && typeof row.s1 === "number"
  )
}

export interface Slice {
  /** Oxyn's key, `c0`… — never the category, which is data. */
  slot: string
  label: string
  value: number
  fill: string
}

/**
 * The categories of the first series, one colour each, in the query's order —
 * or largest first, for a pie: a pie is read by comparing neighbours, and it
 * is the one place a chart departs from the query's order. Colours follow the
 * final order, so a pie's largest part always wears `--chart-1`.
 */
export function slicesOf(
  data: ChartData,
  order: "query" | "share"
): Array<Slice> {
  const slices = data.rows.map((row) => ({
    label: String(row.axis ?? ""),
    value: typeof row.s0 === "number" ? row.s0 : 0,
  }))
  if (order === "share") slices.sort((a, b) => b.value - a.value)
  return slices.map((slice, index) => ({
    ...slice,
    slot: `c${index}`,
    fill: `var(--color-c${index})`,
  }))
}
