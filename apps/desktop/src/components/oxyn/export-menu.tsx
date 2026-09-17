import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { CancelCircleIcon, Download04Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Spinner } from "@/components/ui/spinner"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { ExportFormatChoice } from "@/lib/ipc/results"

/** Where an export stands: from the save dialog to the last byte written. */
export type ExportState =
  | { status: "idle" }
  | { status: "exporting"; format: string; cancelling: boolean }

/**
 * The export menu of a result, by props.
 *
 * A disabled trigger stays focusable so that its reason can be read, by
 * pointer and by keyboard: a `title` on a disabled button is never shown
 * (docs/UX-SPEC.md, « Ce qui est exporté est ce qui est affiché »). A format
 * the product cannot write yet is listed unavailable, never hidden and never
 * offered.
 */
export function ExportMenuView({
  formats,
  formatsFailed,
  exportable,
  reason,
  state,
  label = "Export",
  scope = "Export the whole result as",
  onExport,
  onCancel,
}: {
  /** `null` while the list loads. */
  formats: Array<ExportFormatChoice> | null
  formatsFailed: boolean
  exportable: boolean
  /** Why the result cannot be exported; read when `exportable` is false. */
  reason: string
  state: ExportState
  label?: string
  scope?: string
  onExport: (format: ExportFormatChoice) => void
  onCancel: () => void
}) {
  const reasonId = React.useId()

  if (state.status === "exporting") {
    return (
      <ButtonGroup aria-label="Export in progress">
        <Button variant="outline" size="sm" disabled>
          <Spinner data-icon="inline-start" />
          {state.cancelling ? "Cancelling…" : `Exporting ${state.format}…`}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={state.cancelling}
          onClick={onCancel}
        >
          <HugeiconsIcon
            icon={CancelCircleIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Cancel export
        </Button>
      </ButtonGroup>
    )
  }

  if (!exportable) {
    // Not `disabled`: a disabled button takes no focus and no hover, and its
    // reason would never be read.
    return (
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="outline"
              size="sm"
              aria-disabled="true"
              aria-describedby={reasonId}
              className="cursor-not-allowed opacity-50 hover:bg-background"
            />
          }
        >
          <HugeiconsIcon
            icon={Download04Icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          {label}
          <span id={reasonId} className="sr-only">
            Unavailable: {reason}
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-72">{reason}</TooltipContent>
      </Tooltip>
    )
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="outline" size="sm" />}>
        <HugeiconsIcon
          icon={Download04Icon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        {label}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-64">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{scope}</DropdownMenuLabel>
          {formats === null && !formatsFailed ? (
            <DropdownMenuItem disabled>
              <Spinner data-icon="inline-start" />
              Reading formats…
            </DropdownMenuItem>
          ) : null}
          {formats?.map((format) => (
            <DropdownMenuItem
              key={format.format}
              disabled={!format.supported}
              onClick={() => onExport(format)}
            >
              <span className="truncate">{format.label}</span>
              <span className="font-mono text-xs text-muted-foreground">
                .{format.extension}
              </span>
              {format.supported ? null : (
                <span className="ml-auto text-xs text-muted-foreground">
                  unavailable
                </span>
              )}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
        {formatsFailed ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              <DropdownMenuLabel className="font-normal text-muted-foreground">
                The export formats could not be read.
              </DropdownMenuLabel>
            </DropdownMenuGroup>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
