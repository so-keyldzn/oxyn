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
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

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

const shapesOf = (family: ChartFamily) =>
  CHART_SHAPES.filter((shape) => SHAPE_FAMILY[shape] === family)

/** Why no shape of a family can draw these rows, or `null` if one can. */
function familyReason(
  family: ChartFamily,
  verdicts: Record<ChartShape, ShapeVerdict>
): string | null {
  const shapes = shapesOf(family)
  if (shapes.some((shape) => verdicts[shape].possible)) return null
  const first = shapes[0] ? verdicts[shapes[0]] : undefined
  return first && !first.possible ? first.reason : null
}

function FamilyItem({
  family,
  reason,
}: {
  family: ChartFamily
  reason: string | null
}) {
  const reasonId = React.useId()
  const label = FAMILY_LABEL[family]
  // A disabled control takes no pointer event: the tooltip sits on a wrapper
  // that is always there, and the reason is also the button's description.
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex" />}>
        <ToggleGroupItem
          value={family}
          aria-label={label}
          aria-describedby={reason ? reasonId : undefined}
          disabled={reason !== null}
        >
          <HugeiconsIcon icon={FAMILY_ICON[family]} strokeWidth={2} />
        </ToggleGroupItem>
      </TooltipTrigger>
      <TooltipContent className="max-w-64">
        {reason ? `${label}: ${reason}` : label}
      </TooltipContent>
      {reason ? (
        <span id={reasonId} className="sr-only">
          {reason}
        </span>
      ) : null}
    </Tooltip>
  )
}

/**
 * The chart's shape: a family, then its variant. « Auto » is Oxyn's choice;
 * every family is listed, and one these rows cannot honestly be drawn by is
 * disabled with its reason — never hidden, so the user learns why.
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
  const family = choice === "auto" ? "auto" : SHAPE_FAMILY[choice]
  const variants = shapesOf(SHAPE_FAMILY[drawn])
  return (
    <div className="flex flex-wrap items-center gap-2">
      <ToggleGroup
        size="sm"
        variant="outline"
        spacing={0}
        aria-label="Chart shape"
        value={[family]}
        onValueChange={(next) => {
          const picked = next[0]
          // Pressing the chosen item again would leave nothing chosen.
          if (picked === undefined || picked === family) return
          if (picked === "auto") return onChoose("auto")
          const shapes = shapesOf(picked as ChartFamily)
          onChoose(shapes.find((shape) => verdicts[shape].possible) ?? drawn)
        }}
      >
        <ToggleGroupItem value="auto" aria-label="Automatic shape">
          Auto
        </ToggleGroupItem>
        {CHART_FAMILIES.map((item) => (
          <FamilyItem
            key={item}
            family={item}
            reason={familyReason(item, verdicts)}
          />
        ))}
      </ToggleGroup>
      {variants.length > 1 ? (
        <Select
          value={drawn}
          onValueChange={(next) => {
            if (typeof next === "string" && next !== drawn) onChoose(next)
          }}
        >
          <SelectTrigger
            size="sm"
            aria-label={`Chart variant: ${SHAPE_LABEL[drawn]}`}
            className="min-w-0"
          >
            <SelectValue className="min-w-0">
              {() => <span className="truncate">{SHAPE_LABEL[drawn]}</span>}
            </SelectValue>
          </SelectTrigger>
          <SelectContent alignItemWithTrigger={false} className="w-72">
            <SelectGroup>
              {variants.map((shape) => {
                const verdict = verdicts[shape]
                return (
                  <SelectItem
                    key={shape}
                    value={shape}
                    disabled={!verdict.possible}
                  >
                    {/* In a list box a tooltip is out of the keyboard's
                        reach: the reason is written under the name. */}
                    <span className="flex flex-col">
                      <span>{SHAPE_LABEL[shape]}</span>
                      {verdict.possible ? null : (
                        <span className="text-xs whitespace-normal text-muted-foreground">
                          {verdict.reason}
                        </span>
                      )}
                    </span>
                  </SelectItem>
                )
              })}
            </SelectGroup>
          </SelectContent>
        </Select>
      ) : null}
    </div>
  )
}
