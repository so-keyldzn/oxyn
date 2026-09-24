import { describe, expect, it } from "vitest"

import { grammarOf, highlight } from "./code-highlight"
import { svgDataUrl, svgSize, withoutConfiguration } from "./mermaid-render"
import { cellNumber, chartData, chartPlan } from "./result-chart-model"
import type { ResultColumn } from "@/lib/ipc/types"

const column = (name: string, dataType: string): ResultColumn => ({
  name,
  dataType,
  nullable: true,
})

describe("a result chart", () => {
  it("needs a date or category axis and a numeric column", () => {
    expect(
      chartPlan(
        [column("day", "Date32"), column("total", "Decimal128(12, 2)")],
        10
      )
    ).toEqual({ axis: 0, axisKind: "temporal", series: [1] })
    expect(
      chartPlan([column("status", "Utf8"), column("n", "Int64")], 10)
    ).toEqual({ axis: 0, axisKind: "category", series: [1] })
    expect(chartPlan([column("a", "Utf8"), column("b", "Utf8")], 10)).toBeNull()
  })

  it("reads back only what is plainly a number", () => {
    // Grouped as Rust groups, by U+00A0, spelled by its code to stay visible.
    const grouped = ["4", "823", "917"].join(String.fromCharCode(0xa0))
    expect(cellNumber(grouped)).toBe(4823917)
    expect(cellNumber("-12.50")).toBe(-12.5)
    expect(cellNumber("1e3")).toBe(1000)
    expect(cellNumber(null)).toBeNull()
    for (const text of [
      "1,234",
      "12 %",
      "NaN",
      "Infinity",
      "0x1F",
      "",
      "1e999",
    ])
      expect(cellNumber(text)).toBeUndefined()
    expect(cellNumber({ text: "123", fullBytes: 400 })).toBeUndefined()
  })

  it("leaves out a column holding anything else, and keeps the query's order", () => {
    const columns = [
      column("status", "Utf8"),
      column("orders", "Int64"),
      column("share", "Decimal128(5, 2)"),
    ]
    const plan = chartPlan(columns, 3)
    expect(plan).not.toBeNull()
    if (!plan) return
    const data = chartData(plan, columns, [
      ["shipped", "12", "n/a"],
      ["pending", null, "4.5"],
      ["new", "3", "1.0"],
    ])
    expect(data?.series).toEqual([{ key: "s0", name: "orders" }])
    expect(data?.skipped).toEqual(["share"])
    expect(data?.rows).toEqual([
      { axis: "shipped", s0: 12 },
      { axis: "pending", s0: null },
      { axis: "new", s0: 3 },
    ])
  })
})

describe("a mermaid block", () => {
  it("loses any configuration it carries", () => {
    const source = [
      "---",
      "config:",
      "  securityLevel: loose",
      "---",
      '%%{init: {"securityLevel": "loose", "htmlLabels": true}}%%',
      "graph TD",
      "  A --> B",
    ].join("\n")
    const { text, stripped } = withoutConfiguration(source)
    expect(stripped).toBe(true)
    expect(text).not.toContain("securityLevel")
    expect(text).toContain("A --> B")
    expect(withoutConfiguration("graph TD\n A --> B").stripped).toBe(false)
  })

  it("becomes a data URL, whatever its characters", () => {
    const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 40"><text>été → 数据</text></svg>'
    const url = svgDataUrl(svg)
    expect(url.startsWith("data:image/svg+xml;base64,")).toBe(true)
    const bytes = Uint8Array.from(
      atob(url.slice("data:image/svg+xml;base64,".length)),
      (char) => char.charCodeAt(0)
    )
    expect(new TextDecoder().decode(bytes)).toBe(svg)
    expect(svgSize(svg)).toEqual({ width: 120, height: 40 })
    expect(svgSize("<svg></svg>")).toBeNull()
  })
})

describe("code colouring", () => {
  it("names a grammar only for the languages it loads", () => {
    expect(grammarOf("PostgreSQL")).toBe("sql")
    expect(grammarOf("yml")).toBe("yaml")
    expect(grammarOf("sh")).toBe("shellscript")
    expect(grammarOf("brainfuck")).toBeNull()
    expect(grammarOf("__proto__")).toBeNull()
  })

  it("returns tokens coloured by the app's variables, text unchanged", async () => {
    const text = "SELECT '<b>x</b>' AS v -- note\nFROM t"
    const lines = await highlight("sql", text)
    expect(lines).not.toBeNull()
    expect(
      lines
        ?.map((line) => line.map((token) => token.content).join(""))
        .join("\n")
    ).toBe(text)
    const colours = new Set(lines?.flat().map((token) => token.color))
    expect(colours).toContain("var(--primary-text)")
    expect(colours).toContain("var(--success)")
    expect(lines?.flat().some((token) => token.italic)).toBe(true)
  })
})
