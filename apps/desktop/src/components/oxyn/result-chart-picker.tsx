import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ChartAreaIcon,
  ChartColumnIcon,
  ChartLineData01Icon,
  ChartRadarIcon,
  ChartRingIcon,
  ChartScatterIcon,
  HashtagIcon,
  MagicWand01Icon,
  PieChartIcon,
} from "@hugeicons/core-free-icons"

import {
  CHART_FAMILIES,
  CHART_SHAPES,
  FAMILY_LABEL,
  SHAPE_FAMILY,
  SHAPE_LABEL,
} from "@/components/oxyn/result-chart-shapes"
import type {
  ChartFamily,
  ChartShape,
  ShapeVerdict,
} from "@/components/oxyn/result-chart-shapes"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

const FAMILY_ICON: Record<ChartFamily, typeof ChartAreaIcon> = {
  kpi: HashtagIcon,
  area: ChartAreaIcon,
  bar: ChartColumnIcon,
  line: ChartLineData01Icon,
  pie: PieChartIcon,
  radar: ChartRadarIcon,
  radial: ChartRingIcon,
  scatter: ChartScatterIcon,
}

/** The shapes of a family these rows can be drawn by. */
const possibleShapes = (
  family: ChartFamily,
  verdicts: Record<ChartShape, ShapeVerdict>
) =>
  CHART_SHAPES.filter(
    (shape) => SHAPE_FAMILY[shape] === family && verdicts[shape].possible
  )

/** Every value's label, for Base UI: typeahead and the selected item's text. */
const ITEMS = [
  { value: "auto", label: "Auto" },
  ...CHART_SHAPES.map((shape) => ({ value: shape, label: SHAPE_LABEL[shape] })),
]

/**
 * The chart's shape, as one menu: the trigger shows only what is drawn, the
 * list, by family, only the shapes these rows can honestly be drawn by.
 * « Auto » is Oxyn's choice.
 */
export function ResultChartPicker({
  choice,
  drawn,
  verdicts,
  onChoose,
}: {
  choice: ChartShape | "auto"
  /** The shape being drawn, whoever chose it. */
  drawn: ChartShape
  verdicts: Record<ChartShape, ShapeVerdict>
  onChoose: (choice: ChartShape | "auto") => void
}) {
  const current =
    choice === "auto" ? `Auto · ${SHAPE_LABEL[drawn]}` : SHAPE_LABEL[drawn]
  return (
    <Select
      items={ITEMS}
      value={choice}
      onValueChange={(next) => {
        if (next !== null && next !== choice) onChoose(next)
      }}
    >
      <SelectTrigger
        size="sm"
        aria-label={`Chart shape: ${current}`}
        className="min-w-0 self-start"
      >
        <SelectValue className="min-w-0">
          {() => (
            <>
              <HugeiconsIcon
                icon={FAMILY_ICON[SHAPE_FAMILY[drawn]]}
                strokeWidth={2}
              />
              <span className="truncate">{current}</span>
            </>
          )}
        </SelectValue>
      </SelectTrigger>
      <SelectContent alignItemWithTrigger={false} className="w-72">
        <SelectGroup>
          <SelectItem value="auto">
            <HugeiconsIcon icon={MagicWand01Icon} strokeWidth={2} />
            Auto
          </SelectItem>
        </SelectGroup>
        {CHART_FAMILIES.map((family) => {
          const shapes = possibleShapes(family, verdicts)
          if (shapes.length === 0) return null
          return (
            <React.Fragment key={family}>
              <SelectSeparator />
              <SelectGroup>
                <SelectLabel>{FAMILY_LABEL[family]}</SelectLabel>
                {shapes.map((shape) => (
                  <SelectItem key={shape} value={shape}>
                    {SHAPE_LABEL[shape]}
                  </SelectItem>
                ))}
              </SelectGroup>
            </React.Fragment>
          )
        })}
      </SelectContent>
    </Select>
  )
}
