import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon } from "@hugeicons/core-free-icons"

import type { FetchPage } from "@/components/oxyn/result-grid"
import { CartesianDrawing } from "@/components/oxyn/result-chart-cartesian"
import { chartData } from "@/components/oxyn/result-chart-model"
import type { ChartData, ChartPlan } from "@/components/oxyn/result-chart-model"
import { ResultChartPicker } from "@/components/oxyn/result-chart-picker"
import { PolarDrawing } from "@/components/oxyn/result-chart-polar"
import {
  SHAPE_FAMILY,
  SHAPE_LABEL,
  autoShape,
  scatterRows,
  shapeToDraw,
  shapeVerdicts,
} from "@/components/oxyn/result-chart-shapes"
import type { ChartShape } from "@/components/oxyn/result-chart-shapes"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Spinner } from "@/components/ui/spinner"
import type { ResultColumn } from "@/lib/ipc/types"

type ChartState =
  | { status: "loading" }
  | { status: "ready"; data: ChartData | null }
  | { status: "expired" }
  | { status: "error"; message: string }

/**
 * Not in shadcn's gallery: one row read as key figures, in the gallery's
 * card-header type scale. The values are Rust's text, not a number redrawn —
 * a figure shown alone must match the grid and the export.
 */
function KeyFigures({ data }: { data: ChartData }) {
  return (
    <dl className="flex flex-wrap gap-x-8 gap-y-3 py-2">
      {data.figures.map((figure) => (
        <div key={figure.key} className="flex min-w-0 flex-col gap-0.5">
          <dt className="truncate text-xs text-muted-foreground">
            {figure.name}
          </dt>
          <dd className="font-mono text-2xl font-semibold break-all tabular-nums">
            {figure.text}
          </dd>
        </div>
      ))}
    </dl>
  )
}

function Drawing({ shape, data }: { shape: ChartShape; data: ChartData }) {
  switch (SHAPE_FAMILY[shape]) {
    case "kpi":
      return <KeyFigures data={data} />
    case "pie":
    case "radar":
    case "radial":
      return <PolarDrawing shape={shape} data={data} />
    default:
      return <CartesianDrawing shape={shape} data={data} />
  }
}

/** What is drawn, said in words: the figure's caption, read by a screen reader. */
function caption(shape: ChartShape, data: ChartData): string {
  const names = data.series.map((series) => series.name)
  const axis = data.axis?.name ?? ""
  const parts: Array<string> = []
  switch (SHAPE_FAMILY[shape]) {
    case "kpi":
      parts.push(`${names.join(", ")}, from the only row the query returned`)
      break
    case "scatter": {
      const [x, y, ...rest] = names
      parts.push(`${y ?? ""} against ${x ?? ""}`)
      const missing = data.rows.length - scatterRows(data).length
      if (missing > 0)
        parts.push(`${missing} rows without both values left out`)
      if (rest.length > 0)
        parts.push(`not drawn by a scatter: ${rest.join(", ")}`)
      break
    }
    case "pie":
      parts.push(`${names.join(", ")} by ${axis}, largest share first`)
      break
    default:
      parts.push(
        `${names.join(", ")} by ${axis}, in the order the query returned`
      )
      if (shape === "area-stacked" || shape === "bar-stacked")
        parts.push("stacked: the top edge is their sum")
      if (shape === "area-expanded")
        parts.push("each point as a share of its row's total")
  }
  if (data.skipped.length > 0)
    parts.push(`left out, not plainly numeric: ${data.skipped.join(", ")}`)
  return parts.join(" · ")
}

/** The rows, drawn by Oxyn's pick or the user's, with the picker above. */
export function ResultChartView({
  data,
  initialChoice = "auto",
}: {
  data: ChartData
  /** For stories: the user's choice already made. */
  initialChoice?: ChartShape | "auto"
}) {
  const [choice, setChoice] = React.useState<ChartShape | "auto">(initialChoice)
  const verdicts = React.useMemo(() => shapeVerdicts(data), [data])
  const auto = React.useMemo(() => autoShape(data, verdicts), [data, verdicts])
  const shape = shapeToDraw(choice, auto, verdicts)
  if (shape === null)
    return (
      <p className="px-3 py-2 text-xs text-muted-foreground">
        Nothing to chart: no shape can draw these rows honestly.
      </p>
    )
  return (
    <figure
      data-slot="assistant-result-chart"
      className="flex min-w-0 flex-col gap-2 px-3 py-2"
    >
      <ResultChartPicker
        choice={choice === "auto" || shape !== choice ? "auto" : choice}
        drawn={shape}
        verdicts={verdicts}
        onChoose={setChoice}
      />
      <Drawing shape={shape} data={data} />
      <figcaption className="text-xs text-muted-foreground">
        {caption(shape, data)}
      </figcaption>
      <p className="sr-only" aria-live="polite">
        {`Drawn as ${SHAPE_LABEL[shape]}${choice === "auto" ? ", chosen by Oxyn" : ""}.`}
      </p>
    </figure>
  )
}

/**
 * The first rows of an agent's result, as a chart — the ones the grid shows,
 * read from the same buffer, and never more than `rows`.
 *
 * Nothing is recomputed but what a shape is: a pie is read largest share
 * first, a donut writes its total, a 100 % stack its shares. Every other shape
 * draws the rows in the query's order; a column that is not plainly numeric is
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
      return <ResultChartView data={state.data} />
  }
}
