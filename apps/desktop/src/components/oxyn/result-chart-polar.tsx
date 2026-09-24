// Pie, radar and radial drawings of an agent's result, each from the matching
// block of shadcn's chart gallery (named on each function), and a legend on
// every one: a category known only by hovering its slice is not written down.

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

const POLAR =
  "mx-auto aspect-square max-h-[250px] min-h-48 [&_.recharts-polar-angle-axis-tick_text]:fill-muted-foreground"

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

const sliceLegend = (
  <ChartLegend
    content={<ChartLegendContent nameKey="slot" />}
    className="-translate-y-2 flex-wrap gap-2 *:basis-1/4 *:justify-center"
  />
)

/** chart-pie-simple, chart-pie-donut, chart-pie-donut-text, with chart-pie-legend. */
function PieDrawing({ shape, data }: { shape: ChartShape; data: ChartData }) {
  const slices = slicesOf(data, "share")
  const total = slices.reduce((sum, slice) => sum + slice.value, 0)
  return (
    <ChartContainer
      config={sliceConfig(data, slices)}
      className="mx-auto aspect-square max-h-[280px] min-h-48"
    >
      <PieChart accessibilityLayer>
        <ChartTooltip
          cursor={false}
          content={<ChartTooltipContent hideLabel nameKey="slot" />}
        />
        <Pie
          data={slices}
          dataKey="value"
          nameKey="slot"
          innerRadius={shape === "pie" ? undefined : 60}
          strokeWidth={shape === "donut-total" ? 5 : undefined}
          isAnimationActive={false}
        >
          {shape === "donut-total" ? (
            <Label
              content={({ viewBox }) => {
                if (viewBox && "cx" in viewBox && "cy" in viewBox)
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
                        className="fill-foreground text-3xl font-bold"
                      >
                        {total.toLocaleString()}
                      </tspan>
                      <tspan
                        x={viewBox.cx}
                        y={viewBox.cy + 24}
                        className="fill-muted-foreground"
                      >
                        {data.series[0]?.name}
                      </tspan>
                    </text>
                  )
                return null
              }}
            />
          ) : null}
        </Pie>
        {sliceLegend}
      </PieChart>
    </ChartContainer>
  )
}

/** chart-radial-simple, with chart-pie-legend's legend. */
function RadialDrawing({ data }: { data: ChartData }) {
  const slices = slicesOf(data, "query")
  return (
    <ChartContainer config={sliceConfig(data, slices)} className={POLAR}>
      <RadialBarChart
        accessibilityLayer
        data={slices}
        innerRadius={30}
        outerRadius={110}
      >
        <ChartTooltip
          cursor={false}
          content={<ChartTooltipContent hideLabel nameKey="slot" />}
        />
        <RadialBar dataKey="value" background isAnimationActive={false} />
        {sliceLegend}
      </RadialBarChart>
    </ChartContainer>
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
