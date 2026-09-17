import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowReloadHorizontalIcon,
  CancelCircleIcon,
} from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import { Spinner } from "@/components/ui/spinner"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

/** The object view's tabs, in the GPUI order. */
export type ObjectTab =
  "data" | "structure" | "indexes" | "constraints" | "relations" | "definition"

export const OBJECT_TABS: ReadonlyArray<{ value: ObjectTab; label: string }> = [
  { value: "data", label: "Data" },
  { value: "structure", label: "Structure" },
  { value: "indexes", label: "Indexes" },
  { value: "constraints", label: "Constraints" },
  { value: "relations", label: "Relations" },
  { value: "definition", label: "DDL" },
]

/**
 * The tab to open on. Data only when the object holds rows this session can
 * read: a disabled tab is never the selected one.
 */
export function initialObjectTab(
  target: "data" | "structure" | "definition" | undefined,
  previewable: boolean
): ObjectTab {
  if (target === "structure" || target === "definition") return target
  return previewable ? "data" : "structure"
}

/**
 * A table or view, by props: its tabs and the panel each one shows.
 *
 * Panels are slots, so that the containers that read the backend stay outside
 * and every state can be drawn from fixtures. The name is text, never markup,
 * and keeps its own direction (`dir="auto"`): a Hebrew table name does not
 * flip the tab bar.
 */
export function ObjectViewFrame({
  name,
  kind,
  tab,
  onTabChange,
  dataUnavailable,
  toolbar,
  notice,
  panels,
  onEscape,
}: {
  name: string
  kind: string
  tab: ObjectTab
  onTabChange: (tab: ObjectTab) => void
  /** Why Data is disabled, or `null` when the object can be previewed. */
  dataUnavailable: string | null
  /** Actions of the tab on screen, at the end of the tab bar. */
  toolbar?: React.ReactNode
  /** Said above every tab: a failure to read what the catalog holds. */
  notice?: React.ReactNode
  panels: Record<ObjectTab, React.ReactNode>
  /** Escape not handled deeper — by a menu, a dialog or a field. */
  onEscape?: () => void
}) {
  const reasonId = React.useId()
  return (
    <Tabs
      onKeyDown={(event: React.KeyboardEvent) => {
        if (event.key === "Escape" && !event.defaultPrevented && onEscape) {
          event.preventDefault()
          onEscape()
        }
      }}
      value={tab}
      onValueChange={(value: unknown) => {
        const next = OBJECT_TABS.find((candidate) => candidate.value === value)
        if (next) onTabChange(next.value)
      }}
      className="flex h-full min-h-0 flex-col gap-0"
    >
      <div className="flex min-h-10 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b px-3 py-1">
        <div className="flex min-w-0 items-baseline gap-2">
          <h2
            dir="auto"
            title={name}
            className="max-w-72 truncate text-sm font-medium"
          >
            {name}
          </h2>
          <span className="shrink-0 text-xs text-muted-foreground">{kind}</span>
        </div>
        <TabsList
          variant="line"
          aria-label={`Views of ${name}`}
          className="max-w-full overflow-x-auto"
        >
          {OBJECT_TABS.map((candidate) =>
            candidate.value === "data" && dataUnavailable ? (
              <Tooltip key={candidate.value}>
                {/* The trigger is the wrapper, not the tab: a disabled button
                    receives no pointer event, so a tooltip rendered on it is
                    never shown. The reason below carries the keyboard. */}
                <TooltipTrigger render={<span className="inline-flex" />}>
                  <TabsTrigger
                    value="data"
                    disabled
                    aria-describedby={reasonId}
                  >
                    Data
                  </TabsTrigger>
                </TooltipTrigger>
                <TooltipContent className="max-w-72">
                  {dataUnavailable}
                </TooltipContent>
              </Tooltip>
            ) : (
              <TabsTrigger key={candidate.value} value={candidate.value}>
                {candidate.label}
              </TabsTrigger>
            )
          )}
        </TabsList>
        {toolbar ? (
          <div className="ml-auto flex shrink-0 items-center gap-2">
            {toolbar}
          </div>
        ) : null}
      </div>
      {dataUnavailable ? (
        // Said once in text too: a tooltip on a disabled tab is not reachable
        // by every reader.
        <p id={reasonId} className="sr-only">
          Data is unavailable: {dataUnavailable}
        </p>
      ) : null}

      {notice}

      {OBJECT_TABS.map((candidate) => (
        <TabsContent
          key={candidate.value}
          value={candidate.value}
          className="flex min-h-0 flex-1 flex-col"
        >
          {panels[candidate.value]}
        </TabsContent>
      ))}
    </Tabs>
  )
}

/**
 * The preview's own actions: what it is, how to export what is shown, and
 * Refresh or Cancel — never both, never one button that toggles between them
 * under the pointer.
 */
export function PreviewToolbar({
  running,
  cancelling,
  exportMenu,
  onRefresh,
  onCancel,
}: {
  running: boolean
  cancelling: boolean
  exportMenu?: React.ReactNode
  onRefresh: () => void
  onCancel: () => void
}) {
  return (
    <>
      <Badge variant="secondary">Read-only preview</Badge>
      {exportMenu}
      <Button
        size="sm"
        variant="outline"
        disabled={running}
        onClick={onRefresh}
      >
        <HugeiconsIcon
          icon={ArrowReloadHorizontalIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        Refresh data
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={!running || cancelling}
        onClick={onCancel}
        aria-keyshortcuts="Escape"
      >
        {cancelling ? (
          <Spinner data-icon="inline-start" />
        ) : (
          <HugeiconsIcon
            icon={CancelCircleIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
        )}
        {cancelling ? "Cancelling…" : "Cancel"}
        {running && !cancelling ? <Kbd>Esc</Kbd> : null}
      </Button>
    </>
  )
}

/** The Relations tab: which way the keys point, then their list. */
export function RelationsPanel({
  direction,
  onDirectionChange,
  children,
}: {
  direction: "incoming" | "outgoing"
  onDirectionChange: (direction: "incoming" | "outgoing") => void
  children: React.ReactNode
}) {
  return (
    <>
      <div className="flex min-h-9 shrink-0 flex-wrap items-center gap-2 border-b px-3 py-1">
        <ToggleGroup
          size="sm"
          variant="outline"
          spacing={0}
          value={[direction]}
          onValueChange={(value: Array<unknown>) => {
            const next = value[0]
            if (next === "outgoing" || next === "incoming")
              onDirectionChange(next)
          }}
          aria-label="Relation direction"
        >
          <ToggleGroupItem value="incoming">Incoming</ToggleGroupItem>
          <ToggleGroupItem value="outgoing">Outgoing</ToggleGroupItem>
        </ToggleGroup>
        <span className="min-w-0 truncate text-xs text-muted-foreground">
          {direction === "incoming"
            ? "Keys other tables declare towards this one."
            : "Keys this table declares towards others."}
        </span>
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </>
  )
}
