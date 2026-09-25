// Pie, radar and radial drawings of an agent's result, each from the matching
// block of shadcn's chart gallery (named on each function), and a legend on
// every one: a category known only by hovering its slice is not written down.

import type { ReactNode } from "react"
import {
  Label,
  Pie,
  PieChart,
  PolarAngleAxis,
  PolarGrid,
  Radar,
  RadarChart,
  RadialBar,
  RadialBarChart,
} from "recharts"

import {
  CHART_COLORS,
  seriesConfig,
} from "@/components/oxyn/result-chart-cartesian"
import type { ChartData } from "@/components/oxyn/result-chart-model"
import { slicesOf } from "@/components/oxyn/result-chart-shapes"
import type { ChartShape, Slice } from "@/components/oxyn/result-chart-shapes"
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart"
import type { ChartConfig } from "@/components/ui/chart"

const POLAR_TICKS =
  "[&_.recharts-polar-angle-axis-tick_text]:fill-muted-foreground"
const POLAR = `mx-auto aspect-square max-h-[250px] min-h-48 ${POLAR_TICKS}`

/**
 * The gallery keys a pie's config by category; here by slot (`c0`…), with the
 * category as its label — a category is data, and `ChartStyle` writes keys
 * into a style sheet.
 */
function sliceConfig(data: ChartData, slices: Array<Slice>): ChartConfig {
  const config: ChartConfig = {
    value: { label: data.series[0]?.name ?? "" },
  }
  slices.forEach((slice, index) => {
    config[slice.slot] = {
      label: slice.label,
      color: CHART_COLORS[index % CHART_COLORS.length],
    }
  })
  return config
}

/**
 * chart-pie-legend's content, written beside the drawing rather than inside
 * it: Recharts takes a legend's height from the plot, so four long labels in
 * the assistant's panel left a donut of radius zero, and laid over the rings of
 * radial bars. Under the drawing in a narrow panel, beside it when there is
 * room; a long label wraps onto two lines, then is cut, and says itself whole
 * in its tooltip and to a screen reader.
 */
function SliceLegend({ slices }: { slices: Array<Slice> }) {
  return (
    <ul className="grid w-full min-w-0 grid-cols-1 gap-x-4 gap-y-1.5 text-xs @xs:grid-cols-2 @md:w-auto @md:flex-1 @md:grid-cols-1">
      {slices.map((slice, index) => (
        <li key={slice.slot} className="flex min-w-0 items-start gap-1.5">
          <span
            aria-hidden
            className="mt-1 size-2 shrink-0 rounded-[2px]"
            style={{
              backgroundColor: CHART_COLORS[index % CHART_COLORS.length],
            }}
          />
          <span title={slice.label} className="line-clamp-2 break-words">
            {slice.label}
          </span>
        </li>
      ))}
    </ul>
  )
}

/** A polar drawing and its legend, which never share a pixel. */
function WithLegend({
  slices,
  children,
}: {
  slices: Array<Slice>
  children: ReactNode
}) {
  return (
    <div className="@container min-w-0">
      <div className="flex flex-col items-center gap-3 @md:flex-row">
        {children}
        <SliceLegend slices={slices} />
      </div>
    </div>
  )
}

/**
 * The gallery's box, sized by its width: Recharts measures it, and a height
 * left to the content would be measured at zero.
 */
const POLAR_BOX = "aspect-square w-full max-w-[250px] min-w-40 shrink-0"

/**
 * The gallery's size for text written in the hole, or less, so that it fits
 * `width`: the hole narrows with the panel, and a total of nine characters in
 * `text-3xl` spilled onto the ring. SVG text does not wrap; 0.62 em is a wide
 * digit of the interface font.
 */
function fittedSize(text: string, width: number, max: number): number {
  return Math.min(max, width / (Math.max(1, text.length) * 0.62))
}

/** chart-pie-simple, chart-pie-donut, chart-pie-donut-text, with chart-pie-legend. */
function PieDrawing({ shape, data }: { shape: ChartShape; data: ChartData }) {
  const slices = slicesOf(data, "share")
  const total = slices.reduce((sum, slice) => sum + slice.value, 0)
  return (
    <WithLegend slices={slices}>
      <ChartContainer config={sliceConfig(data, slices)} className={POLAR_BOX}>
        <PieChart accessibilityLayer>
          <ChartTooltip
            cursor={false}
            content={<ChartTooltipContent hideLabel nameKey="slot" />}
          />
          <Pie
            data={slices}
            dataKey="value"
            nameKey="slot"
            // The gallery's 60 px, as a share of the radius: a fixed hole is
            // wider than the ring in a box narrower than the gallery's.
            innerRadius={shape === "pie" ? undefined : "48%"}
            strokeWidth={shape === "donut-total" ? 5 : undefined}
            isAnimationActive={false}
          >
            {shape === "donut-total" ? (
              <Label
                content={({ viewBox }) => {
                  if (!(viewBox && "cx" in viewBox && "cy" in viewBox))
                    return null
                  const figure = total.toLocaleString()
                  const name = data.series[0]?.name ?? ""
                  // Recharts hands the label no radius (`innerRadius` is 0):
                  // the box is square, so `cx` is half its side, the outer
                  // radius 80 % of it, the hole 48 % of that; the text keeps
                  // a margin inside the hole's diameter.
                  const hole = viewBox.cx * 0.8 * 0.48 * 2 * 0.85
                  const size = fittedSize(figure, hole, 30)
                  return (
                    <text
                      x={viewBox.cx}
                      y={viewBox.cy}
                      textAnchor="middle"
                      dominantBaseline="middle"
                    >
                      <tspan
                        x={viewBox.cx}
                        y={viewBox.cy}
                        fontSize={size}
                        className="fill-foreground font-bold"
                      >
                        {figure}
                      </tspan>
                      <tspan
                        x={viewBox.cx}
                        y={viewBox.cy + size / 2 + 10}
                        fontSize={fittedSize(name, hole, 12)}
                        className="fill-muted-foreground"
                      >
                        {name}
                      </tspan>
                    </text>
                  )
                }}
              />
            ) : null}
          </Pie>
        </PieChart>
      </ChartContainer>
    </WithLegend>
  )
}

/** chart-radial-simple, with chart-pie-legend's legend. */
function RadialDrawing({ data }: { data: ChartData }) {
  const slices = slicesOf(data, "query")
  return (
    <WithLegend slices={slices}>
      <ChartContainer
        config={sliceConfig(data, slices)}
        className={`${POLAR_BOX} ${POLAR_TICKS}`}
      >
        <RadialBarChart
          accessibilityLayer
          data={slices}
          // The gallery's 30 and 110 px of a 250 px box, as shares of it.
          innerRadius="24%"
          outerRadius="88%"
        >
          <ChartTooltip
            cursor={false}
            content={<ChartTooltipContent hideLabel nameKey="slot" />}
          />
          <RadialBar dataKey="value" background isAnimationActive={false} />
        </RadialBarChart>
      </ChartContainer>
    </WithLegend>
  )
}

/** chart-radar-default, chart-radar-legend for several series. */
function RadarDrawing({ data }: { data: ChartData }) {
  return (
    <ChartContainer config={seriesConfig(data)} className={POLAR}>
      <RadarChart accessibilityLayer data={data.rows}>
        <ChartTooltip
          cursor={false}
          content={<ChartTooltipContent indicator="line" />}
        />
        <PolarAngleAxis dataKey="axis" />
        <PolarGrid />
        {data.series.map((series) => (
          <Radar
            key={series.key}
            dataKey={series.key}
            fill={`var(--color-${series.key})`}
            // The gallery leaves the second radar opaque; with more than one,
            // each stays see-through so that none hides another.
            fillOpacity={data.series.length === 1 ? 0.6 : 0.3}
            stroke={`var(--color-${series.key})`}
            isAnimationActive={false}
          />
        ))}
        {data.series.length > 1 ? (
          <ChartLegend content={<ChartLegendContent />} />
        ) : null}
      </RadarChart>
    </ChartContainer>
  )
}

export function PolarDrawing({
  shape,
  data,
}: {
  shape: ChartShape
  data: ChartData
}) {
  if (shape === "radar") return <RadarDrawing data={data} />
  if (shape === "radial") return <RadialDrawing data={data} />
  return <PieDrawing shape={shape} data={data} />
}
