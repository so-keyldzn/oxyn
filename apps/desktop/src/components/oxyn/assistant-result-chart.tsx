import * as React from "react"
import {
  Bar,
  BarChart,
  CartesianGrid,
  Line,
  LineChart,
  XAxis,
  YAxis,
} from "recharts"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon } from "@hugeicons/core-free-icons"

import type { FetchPage } from "@/components/oxyn/result-grid"
import { chartData } from "@/components/oxyn/result-chart-model"
import type { ChartData, ChartPlan } from "@/components/oxyn/result-chart-model"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart"
import type { ChartConfig } from "@/components/ui/chart"
import { Spinner } from "@/components/ui/spinner"
import type { ResultColumn } from "@/lib/ipc/types"

const COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
]

type ChartState =
  | { status: "loading" }
  | { status: "ready"; data: ChartData | null }
  | { status: "expired" }
  | { status: "error"; message: string }

function Drawing({ data }: { data: ChartData }) {
  // The keys are Oxyn's (`s0`, `s1`…), never a column name: `ChartStyle`
  // writes them into a style sheet, where a hostile name would be CSS. The
  // names are labels, drawn as React text by the legend and the tooltip.
  const config: ChartConfig = Object.fromEntries(
    data.series.map((series, index) => [
      series.key,
      { label: series.name, color: COLORS[index % COLORS.length] },
    ])
  )
  const axes = (
    <>
      <CartesianGrid vertical={false} />
      <XAxis
        dataKey="axis"
        tickLine={false}
        axisLine={false}
        tickMargin={8}
        minTickGap={24}
      />
      <YAxis tickLine={false} axisLine={false} width={56} />
      <ChartTooltip content={<ChartTooltipContent />} />
      {data.series.length > 1 ? (
        <ChartLegend content={<ChartLegendContent />} />
      ) : null}
    </>
  )
  return (
    <ChartContainer config={config} className="aspect-auto h-56 w-full">
      {data.kind === "line" ? (
        <LineChart data={data.rows} accessibilityLayer>
          {axes}
          {data.series.map((series) => (
            <Line
              key={series.key}
              dataKey={series.key}
              type="monotone"
              stroke={`var(--color-${series.key})`}
              strokeWidth={2}
              dot={false}
              connectNulls={false}
              isAnimationActive={false}
            />
          ))}
        </LineChart>
      ) : (
        <BarChart data={data.rows} accessibilityLayer>
          {axes}
          {data.series.map((series) => (
            <Bar
              key={series.key}
              dataKey={series.key}
              fill={`var(--color-${series.key})`}
              radius={2}
              isAnimationActive={false}
            />
          ))}
        </BarChart>
      )}
    </ChartContainer>
  )
}

/**
 * The first rows of an agent's result, as a chart — the ones the grid shows,
 * read from the same buffer, and never more than `rows`.
 *
 * Nothing is recomputed: no sorting, no aggregation. What is drawn is what
 * the query returned, in its order; a column that is not plainly numeric is
 * left out and named.
 */
export function AssistantResultChart({
  plan,
  columns,
  rows,
  fetchPage,
}: {
  plan: ChartPlan
  columns: Array<ResultColumn>
  /** Rows to read, already bounded by the caller. */
  rows: number
  fetchPage: FetchPage
}) {
  const [state, setState] = React.useState<ChartState>({ status: "loading" })

  React.useEffect(() => {
    let live = true
    setState({ status: "loading" })
    fetchPage(0, rows).then(
      (answer) => {
        if (!live) return
        setState(
          answer.type === "expired"
            ? { status: "expired" }
            : {
                status: "ready",
                data: chartData(plan, columns, answer.rows.slice(0, rows)),
              }
        )
      },
      (error: unknown) => {
        if (live)
          setState({
            status: "error",
            message: error instanceof Error ? error.message : String(error),
          })
      }
    )
    return () => {
      live = false
    }
  }, [fetchPage, rows, plan, columns])

  switch (state.status) {
    case "loading":
      return (
        <p
          aria-busy
          className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground"
        >
          <Spinner aria-hidden />
          Reading the rows to chart…
        </p>
      )
    case "expired":
      return (
        <p className="px-3 py-2 text-xs text-muted-foreground">
          Result no longer available: Oxyn released these rows. Nothing is rerun
          to bring them back.
        </p>
      )
    case "error":
      return (
        <div className="p-2">
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>
              The rows could not be read. Nothing was rerun.
            </AlertTitle>
            <AlertDescription>
              <p dir="auto" className="font-mono break-words">
                {state.message}
              </p>
            </AlertDescription>
          </Alert>
        </div>
      )
    case "ready":
      if (state.data === null)
        return (
          <p className="px-3 py-2 text-xs text-muted-foreground">
            Nothing to chart: no numeric column holds plain numbers in these
            rows.
          </p>
        )
      return (
        <figure
          data-slot="assistant-result-chart"
          className="flex min-w-0 flex-col gap-1 px-3 py-2"
        >
          <Drawing data={state.data} />
          <figcaption className="text-xs text-muted-foreground">
            {state.data.series.map((series) => series.name).join(", ")} by{" "}
            {state.data.axisName}, in the order the query returned
            {state.data.skipped.length > 0
              ? ` · left out, not plainly numeric: ${state.data.skipped.join(", ")}`
              : null}
          </figcaption>
        </figure>
      )
  }
}
