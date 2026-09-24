// Area, bar, line and scatter drawings of an agent's result. Each starts from
// the matching block of shadcn's chart gallery (named on each function) and
// keeps its structure: no vertical grid, no axis line, no tick line.
//
// Where it departs, for the same reason each time — the reader is reading
// data, not a mood board: curves are `monotone`, which never overshoots a
// value, where the gallery writes `natural`, which can draw a peak or a dip
// the rows do not hold; value axes stay, where the gallery hides some, since
// a bar without a scale is a proportion without a unit; category labels are
// never cut to three letters.

import * as React from "react"
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  LabelList,
  Line,
  LineChart,
  Scatter,
  ScatterChart,
  XAxis,
  YAxis,
} from "recharts"

import type { ChartData } from "@/components/oxyn/result-chart-model"
import { scatterRows } from "@/components/oxyn/result-chart-shapes"
import type { ChartShape } from "@/components/oxyn/result-chart-shapes"
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart"
import type { ChartConfig } from "@/components/ui/chart"

/** The fixed palette, `--chart-1` to `--chart-5`, set for AA in `styles.css`. */
export const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
]

/**
 * The keys are Oxyn's (`s0`, `s1`…), never a column name: `ChartStyle`
 * writes them into a style sheet, where a hostile name would be CSS. The
 * names are labels, drawn as React text by the legend and the tooltip.
 */
export function seriesConfig(data: ChartData): ChartConfig {
  return Object.fromEntries(
    data.series.map((series, index) => [
      series.key,
      { label: series.name, color: CHART_COLORS[index % CHART_COLORS.length] },
    ])
  )
}

const CARTESIAN = "aspect-auto h-56 min-h-56 w-full"

/** A category label longer than this is cut on the axis, whole in the tooltip. */
const AXIS_LABEL = 18

function shortLabel(value: unknown): string {
  const text = String(value)
  return text.length > AXIS_LABEL ? `${text.slice(0, AXIS_LABEL - 1)}…` : text
}

function legend(data: ChartData) {
  return data.series.length > 1 ? (
    <ChartLegend content={<ChartLegendContent />} />
  ) : null
}

function categoryAxis() {
  return (
    <XAxis
      dataKey="axis"
      tickLine={false}
      axisLine={false}
      tickMargin={8}
      minTickGap={24}
      tickFormatter={shortLabel}
    />
  )
}

function valueAxis(percent = false) {
  return (
    <YAxis
      tickLine={false}
      axisLine={false}
      width={56}
      tickFormatter={
        percent ? (value: number) => `${Math.round(value * 100)} %` : undefined
      }
    />
  )
}

/** chart-area-default, -stacked, -stacked-expand, -step, -gradient. */
function AreaDrawing({ shape, data }: { shape: ChartShape; data: ChartData }) {
  // A gradient's id is a document-wide name: built from React's id, never
  // from a column, and stripped of the characters `url(#…)` would misread.
  const gradient = React.useId().replace(/[^a-zA-Z0-9_-]/g, "")
  const stacked = shape === "area-stacked" || shape === "area-expanded"
  return (
    <ChartContainer config={seriesConfig(data)} className={CARTESIAN}>
      <AreaChart
        accessibilityLayer
        data={data.rows}
        margin={{ left: 12, right: 12, top: 12 }}
        stackOffset={shape === "area-expanded" ? "expand" : undefined}
      >
        <CartesianGrid vertical={false} />
        {categoryAxis()}
        {valueAxis(shape === "area-expanded")}
        <ChartTooltip
          cursor={false}
          content={<ChartTooltipContent indicator={stacked ? "dot" : "line"} />}
        />
        {legend(data)}
        {shape === "area-gradient" ? (
          <defs>
            {data.series.map((series) => (
              <linearGradient
                key={series.key}
                id={`${gradient}-${series.key}`}
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="5%"
                  stopColor={`var(--color-${series.key})`}
                  stopOpacity={0.8}
                />
                <stop
                  offset="95%"
                  stopColor={`var(--color-${series.key})`}
                  stopOpacity={0.1}
                />
              </linearGradient>
            ))}
          </defs>
        ) : null}
        {data.series.map((series) => (
          <Area
            key={series.key}
            dataKey={series.key}
            type={shape === "area-step" ? "step" : "monotone"}
            fill={
              shape === "area-gradient"
                ? `url(#${gradient}-${series.key})`
                : `var(--color-${series.key})`
            }
            fillOpacity={0.4}
            stroke={`var(--color-${series.key})`}
            // The gallery's gradient block stacks its areas; here stacking is
            // its own choice, so that a sum is never drawn unannounced.
            stackId={stacked ? "a" : undefined}
            connectNulls={false}
            isAnimationActive={false}
          />
        ))}
      </AreaChart>
    </ChartContainer>
  )
}

/** chart-line-linear, -default (curved), -step, -dots; -multiple for several. */
function LineDrawing({ shape, data }: { shape: ChartShape; data: ChartData }) {
  return (
    <ChartContainer config={seriesConfig(data)} className={CARTESIAN}>
      <LineChart
        accessibilityLayer
        data={data.rows}
        margin={{ left: 12, right: 12 }}
      >
        <CartesianGrid vertical={false} />
        {categoryAxis()}
        {valueAxis()}
        <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
        {legend(data)}
        {data.series.map((series) => (
          <Line
            key={series.key}
            dataKey={series.key}
            type={
              shape === "line-linear"
                ? "linear"
                : shape === "line-step"
                  ? "step"
                  : "monotone"
            }
            stroke={`var(--color-${series.key})`}
            strokeWidth={2}
            dot={
              shape === "line-dots"
                ? { fill: `var(--color-${series.key})` }
                : false
            }
            activeDot={shape === "line-dots" ? { r: 6 } : undefined}
            connectNulls={false}
            isAnimationActive={false}
          />
        ))}
      </LineChart>
    </ChartContainer>
  )
}

/** Height of one category in horizontal bars: a line of text, and air. */
const BAND = 28

/** chart-bar-horizontal. Tall enough for every label, scrolled past 320 px. */
function HorizontalBars({ data }: { data: ChartData }) {
  const height = Math.max(224, data.rows.length * BAND + 48)
  const scrolls = height > 320
  return (
    <div
      className="max-h-80 overflow-y-auto"
      // A region that scrolls must be reachable from the keyboard.
      tabIndex={scrolls ? 0 : undefined}
      role={scrolls ? "group" : undefined}
      aria-label={scrolls ? "Chart, scrollable" : undefined}
    >
      <ChartContainer
        config={seriesConfig(data)}
        className="aspect-auto w-full"
        style={{ height }}
      >
        <BarChart
          accessibilityLayer
          data={data.rows}
          layout="vertical"
          margin={{ left: 0, right: 12 }}
        >
          <XAxis type="number" tickLine={false} axisLine={false} />
          <YAxis
            dataKey="axis"
            type="category"
            tickLine={false}
            tickMargin={10}
            axisLine={false}
            width={120}
            interval={0}
            tickFormatter={shortLabel}
          />
          <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
          {legend(data)}
          {data.series.map((series) => (
            <Bar
              key={series.key}
              dataKey={series.key}
              fill={`var(--color-${series.key})`}
              radius={5}
              isAnimationActive={false}
            />
          ))}
        </BarChart>
      </ChartContainer>
    </div>
  )
}

/** chart-bar-default, -multiple, -stacked, -label. */
function BarDrawing({ shape, data }: { shape: ChartShape; data: ChartData }) {
  if (shape === "bar-horizontal") return <HorizontalBars data={data} />
  const last = data.series.length - 1
  const radius = (index: number): number | [number, number, number, number] => {
    if (shape === "bar-stacked")
      return index === last ? [4, 4, 0, 0] : index === 0 ? [0, 0, 4, 4] : 0
    return shape === "bar-grouped" ? 4 : 8
  }
  return (
    <ChartContainer config={seriesConfig(data)} className={CARTESIAN}>
      <BarChart
        accessibilityLayer
        data={data.rows}
        margin={shape === "bar-label" ? { top: 20 } : undefined}
      >
        <CartesianGrid vertical={false} />
        {categoryAxis()}
        {shape === "bar-label" ? null : valueAxis()}
        <ChartTooltip
          cursor={false}
          content={
            <ChartTooltipContent
              indicator={shape === "bar-grouped" ? "dashed" : "dot"}
            />
          }
        />
        {legend(data)}
        {data.series.map((series, index) => (
          <Bar
            key={series.key}
            dataKey={series.key}
            fill={`var(--color-${series.key})`}
            radius={radius(index)}
            stackId={shape === "bar-stacked" ? "a" : undefined}
            isAnimationActive={false}
          >
            {shape === "bar-label" ? (
              <LabelList
                position="top"
                offset={12}
                className="fill-foreground"
                fontSize={12}
              />
            ) : null}
          </Bar>
        ))}
      </BarChart>
    </ChartContainer>
  )
}

/**
 * Not in shadcn's gallery: a Recharts scatter written the gallery's way —
 * `ChartContainer`, shadcn's tooltip, the palette. The first numeric column
 * runs along, the second up.
 */
function ScatterDrawing({ data }: { data: ChartData }) {
  const [x, y] = data.series
  if (!x || !y) return null
  return (
    <ChartContainer config={seriesConfig(data)} className={CARTESIAN}>
      <ScatterChart
        accessibilityLayer
        margin={{ left: 12, right: 12, top: 12 }}
      >
        <CartesianGrid vertical={false} />
        <XAxis
          type="number"
          dataKey={x.key}
          name={x.key}
          tickLine={false}
          axisLine={false}
          tickMargin={8}
        />
        <YAxis
          type="number"
          dataKey={y.key}
          name={y.key}
          tickLine={false}
          axisLine={false}
          width={56}
        />
        <ChartTooltip
          cursor={false}
          content={<ChartTooltipContent hideLabel />}
        />
        <Scatter
          data={scatterRows(data)}
          fill={`var(--color-${x.key})`}
          isAnimationActive={false}
        />
      </ScatterChart>
    </ChartContainer>
  )
}

export function CartesianDrawing({
  shape,
  data,
}: {
  shape: ChartShape
  data: ChartData
}) {
  if (shape === "scatter") return <ScatterDrawing data={data} />
  if (shape.startsWith("area")) return <AreaDrawing shape={shape} data={data} />
  if (shape.startsWith("line")) return <LineDrawing shape={shape} data={data} />
  return <BarDrawing shape={shape} data={data} />
}
