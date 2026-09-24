import { describe, expect, it } from "vitest"

import { chartData, chartPlan } from "./result-chart-model"
import {
  CHART_SHAPES,
  autoShape,
  scatterRows,
  shapeToDraw,
  shapeVerdicts,
  slicesOf,
} from "./result-chart-shapes"
import type { ChartShape } from "./result-chart-shapes"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

const column = (name: string, dataType: string): ResultColumn => ({
  name,
  dataType,
  nullable: true,
})

/** The data a chart would draw from these columns and rows. */
function data(columns: Array<ResultColumn>, rows: Array<Array<Cell>>) {
  const plan = chartPlan(columns, rows.length)
  if (!plan) throw new Error("no plan")
  const read = chartData(plan, columns, rows)
  if (!read) throw new Error("no data")
  return read
}

function choose(columns: Array<ResultColumn>, rows: Array<Array<Cell>>) {
  const read = data(columns, rows)
  const verdicts = shapeVerdicts(read)
  return { read, verdicts, auto: autoShape(read, verdicts) }
}

const possible = (
  verdicts: ReturnType<typeof shapeVerdicts>,
  shape: ChartShape
) => verdicts[shape].possible

const reason = (
  verdicts: ReturnType<typeof shapeVerdicts>,
  shape: ChartShape
) => {
  const verdict = verdicts[shape]
  return verdict.possible ? null : verdict.reason
}

const day = column("day", "Date32")
const status = column("status", "Utf8")
const orders = column("orders", "Int64")
const revenue = column("revenue", "Decimal128(12, 2)")

const days = (count: number, value: (index: number) => Cell = String) =>
  Array.from({ length: count }, (_, index): Array<Cell> => [
    `2026-09-${String(index + 1).padStart(2, "0")}`,
    value(index),
  ])

describe("which columns can be charted", () => {
  it("needs a number, and an axis unless two numbers or one row", () => {
    expect(chartPlan([orders], 5)).toBeNull()
    expect(chartPlan([orders], 1)).toEqual({
      axis: null,
      axisKind: null,
      series: [0],
    })
    expect(chartPlan([orders, revenue], 5)).toEqual({
      axis: null,
      axisKind: null,
      series: [0, 1],
    })
    expect(chartPlan([status, day, orders], 5)).toMatchObject({
      axis: 1,
      axisKind: "temporal",
    })
  })

  it("offers nothing for zero rows", () => {
    expect(chartPlan([status, orders], 0)).toBeNull()
  })
})

describe("the shape Oxyn picks", () => {
  it("reads one row as key figures, in Rust's own text", () => {
    const GROUP = String.fromCharCode(0xa0)
    const { read, auto } = choose([orders], [[`4${GROUP}823`]])
    expect(auto).toBe("kpi")
    expect(read.figures).toEqual([
      { key: "s0", name: "orders", text: `4${GROUP}823` },
    ])
  })

  it("fills an area for one series over dates, lines for several", () => {
    expect(choose([day, orders], days(10)).auto).toBe("area")
    const several = days(10).map((row) => [...row, "3"])
    expect(choose([day, orders, revenue], several).auto).toBe("line-curved")
  })

  it("draws a donut for 2 to 5 positive, distinct parts", () => {
    const rows: Array<Array<Cell>> = [
      ["shipped", "12"],
      ["pending", "4"],
      ["new", "7"],
    ]
    expect(choose([status, orders], rows).auto).toBe("donut")
  })

  it("stands bars up for a few categories, grouped when there are several series", () => {
    const six = ["a", "b", "c", "d", "e", "f"].map((label, index) => [
      label,
      String(index + 1),
    ])
    expect(choose([status, orders], six).auto).toBe("bar-vertical")
    const grouped = six.map((row) => [...row, "2"])
    expect(choose([status, orders, revenue], grouped).auto).toBe("bar-grouped")
  })

  it("lays bars down for many categories or long labels", () => {
    const many = Array.from({ length: 13 }, (_, index) => [
      `c${index}`,
      String(index + 1),
    ])
    expect(choose([status, orders], many).auto).toBe("bar-horizontal")
    const long = [
      ["a category named at length", "-1"],
      ["short", "2"],
    ]
    expect(choose([status, orders], long).auto).toBe("bar-horizontal")
  })

  it("sets two numbers against each other when there is no axis", () => {
    const rows = [
      ["1", "2"],
      ["3", "4"],
      ["5", null],
    ]
    const { read, auto } = choose([orders, revenue], rows)
    expect(auto).toBe("scatter")
    expect(scatterRows(read)).toEqual([
      { s0: 1, s1: 2 },
      { s0: 3, s1: 4 },
    ])
  })

  it("never picks a radar, even where one is possible", () => {
    const rows = ["a", "b", "c", "d"].map((label) => [label, "1", "2"])
    const { verdicts, auto } = choose([status, orders, revenue], rows)
    expect(possible(verdicts, "radar")).toBe(true)
    expect(auto).toBe("bar-grouped")
  })

  it("picks nothing when no shape is possible", () => {
    // Two numeric columns, but no row holds both: not even a scatter.
    const { auto } = choose(
      [orders, revenue],
      [
        ["1", null],
        [null, "2"],
      ]
    )
    expect(auto).toBeNull()
  })

  it("keeps the user's choice only while the rows allow it", () => {
    const { verdicts, auto } = choose([day, orders], days(4))
    expect(shapeToDraw("bar-horizontal", auto, verdicts)).toBe("bar-horizontal")
    expect(shapeToDraw("pie", auto, verdicts)).toBe("area")
    expect(shapeToDraw("auto", auto, verdicts)).toBe("area")
  })
})

describe("what a shape refuses, and why", () => {
  const parts = (values: Array<Cell>) =>
    values.map((value, index): Array<Cell> => [`part ${index}`, value])

  it("keeps a pie from a NULL, a negative, a zero, or a repeated category", () => {
    const withNull = choose([status, orders], parts(["3", null, "2"]))
    expect(reason(withNull.verdicts, "pie")).toMatch(/NULL/)
    expect(withNull.auto).not.toBe("donut")
    const negative = choose([status, orders], parts(["3", "-1", "2"]))
    expect(reason(negative.verdicts, "donut")).toMatch(/negative/)
    expect(negative.auto).toBe("bar-vertical")
    const zero = choose([status, orders], parts(["3", "0", "2"]))
    expect(reason(zero.verdicts, "donut-total")).toMatch(/zero/)
    const twice = choose(
      [status, orders],
      [
        ["a", "1"],
        ["a", "2"],
      ]
    )
    expect(reason(twice.verdicts, "pie")).toMatch(/appears twice/)
  })

  it("keeps a pie to the palette, and to more than one part", () => {
    const six = choose([status, orders], parts(["1", "2", "3", "4", "5", "6"]))
    expect(reason(six.verdicts, "pie")).toMatch(/palette has 5/)
    const one = choose([status, orders], parts(["1", "2"]).slice(0, 1))
    // One row is a key figure; a pie of it would be the whole.
    expect(one.auto).toBe("kpi")
    expect(reason(one.verdicts, "pie")).toMatch(/whole/)
  })

  it("keeps a pie, a radial and a radar off a date axis", () => {
    const { verdicts } = choose(
      [day, orders],
      days(4, () => "2")
    )
    for (const shape of ["pie", "radial", "radar"] as const)
      expect(reason(verdicts, shape)).toMatch(/dates/)
  })

  it("draws no line between categories, nor with one point", () => {
    const { verdicts } = choose([status, orders], parts(["1", "2"]))
    expect(reason(verdicts, "line-linear")).toMatch(/trend/)
    expect(reason(verdicts, "area")).toMatch(/trend/)
    const single = choose([day, orders], days(1))
    expect(reason(single.verdicts, "line-curved")).toMatch(/two points/)
  })

  it("stacks only several series of non-negative, known values", () => {
    const one = choose([day, orders], days(4))
    expect(reason(one.verdicts, "area-stacked")).toMatch(/two numeric/)
    const signed = days(4).map((row, index) => [
      ...row,
      index === 2 ? "-3" : "1",
    ])
    const negative = choose([day, orders, revenue], signed)
    expect(reason(negative.verdicts, "bar-stacked")).toMatch(/negative/)
    expect(possible(negative.verdicts, "bar-grouped")).toBe(true)
    const holes = days(4).map((row, index) => [...row, index ? "1" : null])
    expect(
      reason(choose([day, orders, revenue], holes).verdicts, "area-expanded")
    ).toMatch(/NULL/)
  })

  it("splits no share out of a row that sums to zero", () => {
    const rows = days(3).map((row, index) => [
      row[0] ?? null,
      index === 1 ? "0" : "1",
      index === 1 ? "0" : "2",
    ])
    const { verdicts } = choose([day, orders, revenue], rows)
    expect(possible(verdicts, "area-stacked")).toBe(true)
    expect(reason(verdicts, "area-expanded")).toMatch(/sums to zero/)
  })

  it("groups several series only, and stands one up only", () => {
    const one = choose([status, orders], parts(["1", "2"]))
    expect(reason(one.verdicts, "bar-grouped")).toMatch(/nothing to group/)
    const two = choose(
      [status, orders, revenue],
      parts(["1", "2"]).map((row) => [...row, "3"])
    )
    expect(reason(two.verdicts, "bar-vertical")).toMatch(/grouped/)
  })

  it("writes values on bars only while they fit", () => {
    const many = Array.from({ length: 13 }, (_, index) => [`c${index}`, "1"])
    expect(
      reason(choose([status, orders], many).verdicts, "bar-label")
    ).toMatch(/13 bars/)
  })

  it("keeps a radar to 3 to 8 categories of known, non-negative values", () => {
    const two = choose([status, orders], parts(["1", "2"]))
    expect(reason(two.verdicts, "radar")).toMatch(/3 to 8/)
    const holes = choose([status, orders], parts(["1", null, "2"]))
    expect(reason(holes.verdicts, "radar")).toMatch(/NULL/)
    const negative = choose([status, orders], parts(["1", "-2", "2"]))
    expect(reason(negative.verdicts, "radar")).toMatch(/centre/)
  })

  it("sets up a scatter only with two numbers", () => {
    const { verdicts } = choose([status, orders], parts(["1", "2"]))
    expect(reason(verdicts, "scatter")).toMatch(/two numeric/)
  })

  it("draws nothing on an axis that is not there", () => {
    const { verdicts } = choose(
      [orders, revenue],
      [
        ["1", "2"],
        ["3", "4"],
      ]
    )
    for (const shape of CHART_SHAPES)
      if (shape !== "scatter" && shape !== "kpi")
        expect(possible(verdicts, shape)).toBe(false)
  })

  it("draws no key figure from several rows", () => {
    const { verdicts } = choose([day, orders], days(2))
    expect(reason(verdicts, "kpi")).toMatch(/one row; these are 2/)
  })

  it("names every refusal", () => {
    const { verdicts } = choose([status, orders], parts(["1", "2"]))
    for (const shape of CHART_SHAPES) {
      const verdict = verdicts[shape]
      if (!verdict.possible) expect(verdict.reason.length).toBeGreaterThan(10)
    }
  })
})

describe("a column that cannot be read", () => {
  it("is left out, and a chart is drawn from the others", () => {
    const { read, auto } = choose(
      [status, orders, revenue],
      [
        ["a", "1", "n/a"],
        ["b", "2", "3"],
      ]
    )
    expect(read.skipped).toEqual(["revenue"])
    expect(read.series).toEqual([{ key: "s0", name: "orders" }])
    expect(auto).toBe("donut")
  })
})

describe("a pie's slices", () => {
  it("are largest first, coloured by rank; radial bars keep the query's order", () => {
    const read = data(
      [status, orders],
      [
        ["small", "1"],
        ["large", "9"],
        ["middle", "4"],
      ]
    )
    expect(slicesOf(read, "share")).toEqual([
      { label: "large", value: 9, slot: "c0", fill: "var(--color-c0)" },
      { label: "middle", value: 4, slot: "c1", fill: "var(--color-c1)" },
      { label: "small", value: 1, slot: "c2", fill: "var(--color-c2)" },
    ])
    expect(slicesOf(read, "query").map((slice) => slice.label)).toEqual([
      "small",
      "large",
      "middle",
    ])
  })

  it("never carry a category in a key", () => {
    const hostile = '"users"; } body { display: none'
    const read = data(
      [status, orders],
      [
        [hostile, "2"],
        ["b", "1"],
      ]
    )
    for (const slice of slicesOf(read, "share"))
      expect(slice.slot).toMatch(/^c\d$/)
  })
})
